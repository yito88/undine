use std::future::Future;
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};

use crate::context::Context;
use crate::error::ExecError;
use crate::step::Step;

/// A heterogeneous concurrency block.
///
/// All child steps are executed concurrently on the current task using
/// wait-all semantics. If any child fails the block fails; the first
/// error encountered is returned.
pub struct Parallel {
    name: String,
    steps: Vec<Step>,
}

impl Parallel {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            steps: Vec::new(),
        }
    }

    pub fn step(mut self, step: Step) -> Self {
        self.steps.push(step);
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// Run all child step bodies concurrently against a shared `&Context`.
    ///
    /// Aggregation rule: a framework error from any child dominates and is
    /// returned as-is (first one wins). Otherwise step-failure counts are
    /// summed across children. Each child already logged its own errors
    /// at detection time, so this layer only tallies.
    pub(crate) async fn execute_bodies(&self, ctx: &Context) -> Result<(), ExecError> {
        let futs: Vec<_> = self.steps.iter().map(|s| s.execute_child(ctx)).collect();
        let results = join_all(futs).await;
        let mut total: u64 = 0;
        for r in results {
            match r {
                Ok(()) => {}
                Err(ExecError::Framework(e)) => return Err(ExecError::Framework(e)),
                Err(ExecError::Step(n)) => total += n,
            }
        }
        if total == 0 {
            Ok(())
        } else {
            Err(ExecError::Step(total))
        }
    }
}

/// Minimal wait-all combinator.
///
/// Polls every still-pending future on each outer wakeup. Not the most
/// efficient scheduler, but correct and keeps Undine free of a `futures`
/// dependency for this first iteration.
pub(crate) async fn join_all<F>(futures: Vec<F>) -> Vec<F::Output>
where
    F: Future,
{
    struct JoinAll<F: Future> {
        futs: Vec<Option<Pin<Box<F>>>>,
        results: Vec<Option<F::Output>>,
    }

    // Futures are individually boxed/pinned; the outer struct has no
    // self-referential invariants and is safe to move.
    impl<F: Future> Unpin for JoinAll<F> {}

    impl<F: Future> Future for JoinAll<F> {
        type Output = Vec<F::Output>;

        fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
            let this = self.as_mut().get_mut();
            let mut all_done = true;
            for i in 0..this.futs.len() {
                if let Some(fut) = this.futs[i].as_mut() {
                    match fut.as_mut().poll(cx) {
                        Poll::Ready(v) => {
                            this.results[i] = Some(v);
                            this.futs[i] = None;
                        }
                        Poll::Pending => all_done = false,
                    }
                }
            }
            if all_done {
                Poll::Ready(this.results.drain(..).map(|r| r.unwrap()).collect())
            } else {
                Poll::Pending
            }
        }
    }

    let n = futures.len();
    let futs: Vec<Option<Pin<Box<F>>>> = futures.into_iter().map(|f| Some(Box::pin(f))).collect();
    let results: Vec<Option<F::Output>> = (0..n).map(|_| None).collect();
    JoinAll { futs, results }.await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn join_all_waits_for_all() {
        let futs = vec![
            Box::pin(async { 1 }) as Pin<Box<dyn Future<Output = i32> + Send>>,
            Box::pin(async { 2 }),
            Box::pin(async { 3 }),
        ];
        let out = join_all(futs).await;
        assert_eq!(out, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn join_all_empty() {
        let futs: Vec<Pin<Box<dyn Future<Output = ()> + Send>>> = vec![];
        let out = join_all(futs).await;
        assert!(out.is_empty());
    }
}
