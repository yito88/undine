use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;

use crate::context::Context;

/// Boxed closure shape for a mutable, top-level step body.
pub type MutableStepFn = Box<
    dyn for<'a> Fn(&'a mut Context) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>
        + Send
        + Sync,
>;

/// Boxed closure shape for a read-only (parallel / concurrent) step body.
pub type ReadOnlyStepFn = Box<
    dyn for<'a> Fn(&'a Context) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>
        + Send
        + Sync,
>;

/// Which body a [`Step`] carries.
pub(crate) enum StepRunner {
    Mutable(MutableStepFn),
    ReadOnly(ReadOnlyStepFn),
    None,
}

/// Execution policy attached to a [`Step`].
#[derive(Debug, Clone)]
pub struct StepPolicy {
    pub workers: usize,
    pub duration: Option<Duration>,
}

impl Default for StepPolicy {
    fn default() -> Self {
        Self {
            workers: 1,
            duration: None,
        }
    }
}

/// A named, async action.
///
/// - [`Step::run`] registers a mutable body suitable for top-level orchestration
///   (setup / validate / cleanup).
/// - [`Step::run_readonly`] registers a read-only body for use inside
///   [`Parallel`](crate::Parallel) or as a concurrent worker.
/// - [`Step::workers`] and [`Step::duration`] shape read-only execution.
pub struct Step {
    name: String,
    runner: StepRunner,
    policy: StepPolicy,
}

impl Step {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            runner: StepRunner::None,
            policy: StepPolicy::default(),
        }
    }

    /// Attach a mutable async function. Used for top-level orchestration.
    pub fn run<F>(mut self, f: F) -> Self
    where
        F: for<'a> Fn(&'a mut Context) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>
            + Send
            + Sync
            + 'static,
    {
        self.runner = StepRunner::Mutable(Box::new(f));
        self
    }

    /// Attach a read-only async function. Required for steps inside
    /// [`Parallel`](crate::Parallel) and for concurrent workers.
    pub fn run_readonly<F>(mut self, f: F) -> Self
    where
        F: for<'a> Fn(&'a Context) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>
            + Send
            + Sync
            + 'static,
    {
        self.runner = StepRunner::ReadOnly(Box::new(f));
        self
    }

    /// Run N workers concurrently. Only meaningful with a read-only body.
    pub fn workers(mut self, n: usize) -> Self {
        self.policy.workers = n;
        self
    }

    /// Each worker loops its body until `d` elapses.
    /// Only meaningful with a read-only body.
    pub fn duration(mut self, d: Duration) -> Self {
        self.policy.duration = Some(d);
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn policy(&self) -> &StepPolicy {
        &self.policy
    }

    /// Execute this step as a top-level scenario node.
    pub(crate) async fn execute_top(&self, ctx: &mut Context) -> Result<()> {
        match &self.runner {
            StepRunner::Mutable(f) => {
                if self.policy.workers != 1 {
                    anyhow::bail!(
                        "step '{}': workers > 1 requires run_readonly",
                        self.name
                    );
                }
                if self.policy.duration.is_some() {
                    anyhow::bail!(
                        "step '{}': duration requires run_readonly",
                        self.name
                    );
                }
                f(ctx).await
            }
            StepRunner::ReadOnly(f) => execute_readonly(&self.name, f, &self.policy, ctx).await,
            StepRunner::None => {
                anyhow::bail!("step '{}' has no function attached", self.name)
            }
        }
    }

    /// Execute this step as a child inside a [`Parallel`](crate::Parallel).
    pub(crate) async fn execute_child(&self, ctx: &Context) -> Result<()> {
        match &self.runner {
            StepRunner::ReadOnly(f) => execute_readonly(&self.name, f, &self.policy, ctx).await,
            StepRunner::Mutable(_) => {
                anyhow::bail!(
                    "step '{}' inside Parallel must use run_readonly",
                    self.name
                );
            }
            StepRunner::None => {
                anyhow::bail!("step '{}' has no function attached", self.name)
            }
        }
    }
}

async fn execute_readonly(
    _name: &str,
    f: &ReadOnlyStepFn,
    policy: &StepPolicy,
    ctx: &Context,
) -> Result<()> {
    let workers = policy.workers.max(1);
    if workers == 1 {
        worker_loop(f, ctx, policy.duration).await
    } else {
        let futs: Vec<_> = (0..workers)
            .map(|_| worker_loop(f, ctx, policy.duration))
            .collect();
        let results = crate::parallel::join_all(futs).await;
        for r in results {
            r?;
        }
        Ok(())
    }
}

async fn worker_loop(
    f: &ReadOnlyStepFn,
    ctx: &Context,
    duration: Option<Duration>,
) -> Result<()> {
    match duration {
        None => f(ctx).await,
        Some(d) => {
            let deadline = tokio::time::Instant::now() + d;
            while tokio::time::Instant::now() < deadline {
                f(ctx).await?;
            }
            Ok(())
        }
    }
}

/// Wraps an async function into a step-compatible closure.
///
/// Works for both [`Step::run`] and [`Step::run_readonly`]: the closure's
/// parameter type is inferred from the builder's expected signature.
///
/// # Example
/// ```ignore
/// async fn setup(ctx: &mut Context) -> Result<()> { Ok(()) }
/// async fn worker(ctx: &Context) -> Result<()> { Ok(()) }
///
/// Step::named("setup").run(step_fn!(setup));
/// Step::named("worker").run_readonly(step_fn!(worker));
/// ```
#[macro_export]
macro_rules! step_fn {
    ($f:expr) => {
        |ctx| Box::pin($f(ctx))
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slot::Slot;

    const VALUE: Slot<i32> = Slot::new("value");

    #[tokio::test]
    async fn mutable_step_executes() {
        let step = Step::named("test").run(|ctx| {
            Box::pin(async move {
                ctx.insert(VALUE, 42);
                Ok(())
            })
        });

        assert_eq!(step.name(), "test");

        let mut ctx = Context::new();
        step.execute_top(&mut ctx).await.unwrap();
        assert_eq!(*ctx.get(VALUE).unwrap(), 42);
    }

    #[tokio::test]
    async fn readonly_step_executes_at_top_level() {
        let step = Step::named("ro").run_readonly(|ctx| {
            Box::pin(async move {
                // read-only: verify we can read metrics handle from ctx
                ctx.runtime().metrics().counter("x", 1);
                Ok(())
            })
        });

        let mut ctx = Context::new();
        step.execute_top(&mut ctx).await.unwrap();
    }

    #[tokio::test]
    async fn mutable_step_with_workers_errors() {
        let step = Step::named("bad")
            .workers(4)
            .run(|_ctx| Box::pin(async move { Ok(()) }));

        let mut ctx = Context::new();
        let err = step.execute_top(&mut ctx).await.unwrap_err();
        assert!(err.to_string().contains("requires run_readonly"));
    }

    #[tokio::test]
    async fn step_without_func_errors() {
        let step = Step::named("empty");
        let mut ctx = Context::new();
        assert!(step.execute_top(&mut ctx).await.is_err());
    }

    #[tokio::test]
    async fn named_readonly_fn_via_step_fn_macro() {
        async fn worker(ctx: &Context) -> Result<()> {
            ctx.runtime().metrics().counter("named", 1);
            Ok(())
        }
        let step = Step::named("named").run_readonly(step_fn!(worker));
        let mut ctx = Context::new();
        step.execute_top(&mut ctx).await.unwrap();
    }
}
