use anyhow::Result;

use crate::context::Context;
use crate::hook::{Hook, HookPoint, StepInfo};
use crate::step::Step;

/// A sequence of [`Step`]s executed in order against a shared [`Context`].
///
/// Optional [`Hook`]s may be attached to observe step lifecycle events.
pub struct Scenario {
    steps: Vec<Step>,
    hooks: Vec<Hook>,
}

impl Scenario {
    pub fn new() -> Self {
        Self {
            steps: Vec::new(),
            hooks: Vec::new(),
        }
    }

    /// Append a step to the scenario (builder pattern).
    pub fn step(mut self, step: Step) -> Self {
        self.steps.push(step);
        self
    }

    /// Attach a hook to the scenario. Hooks run in insertion order.
    pub fn hook(mut self, hook: Hook) -> Self {
        self.hooks.push(hook);
        self
    }

    /// Execute all steps sequentially. Stops at the first failure,
    /// including hook failures.
    pub async fn run(self) -> Result<()> {
        let mut ctx = Context::new();
        for step in &self.steps {
            let info = StepInfo { name: step.name() };

            for hook in &self.hooks {
                if hook.point() == HookPoint::BeforeStep {
                    (hook.func)(&mut ctx, &info).await?;
                }
            }

            step.execute(&mut ctx).await?;

            for hook in &self.hooks {
                if hook.point() == HookPoint::AfterStep {
                    (hook.func)(&mut ctx, &info).await?;
                }
            }
        }
        Ok(())
    }
}

impl Default for Scenario {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::Hook;
    use crate::slot::Slot;

    const COUNTER: Slot<i32> = Slot::new("counter");
    const EVENTS: Slot<Vec<String>> = Slot::new("events");

    fn record_step(ctx: &mut Context, label: &str) {
        if !ctx.contains(EVENTS) {
            ctx.insert(EVENTS, Vec::<String>::new());
        }
        ctx.get_mut(EVENTS).unwrap().push(label.to_string());
    }

    #[tokio::test]
    async fn scenario_runs_steps_in_order() {
        let result = Scenario::new()
            .step(Step::named("init").run(|ctx| {
                Box::pin(async move {
                    ctx.insert(COUNTER, 0);
                    Ok(())
                })
            }))
            .step(Step::named("increment").run(|ctx| {
                Box::pin(async move {
                    let val = *ctx.get(COUNTER)?;
                    ctx.insert(COUNTER, val + 1);
                    Ok(())
                })
            }))
            .step(Step::named("check").run(|ctx| {
                Box::pin(async move {
                    let val = *ctx.get(COUNTER)?;
                    assert_eq!(val, 1);
                    Ok(())
                })
            }))
            .run()
            .await;

        result.unwrap();
    }

    #[tokio::test]
    async fn scenario_stops_on_failure() {
        let result = Scenario::new()
            .step(Step::named("fail").run(|_ctx| {
                Box::pin(async move { anyhow::bail!("intentional failure") })
            }))
            .step(Step::named("never_reached").run(|ctx| {
                Box::pin(async move {
                    ctx.insert(COUNTER, 999);
                    Ok(())
                })
            }))
            .run()
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn empty_scenario_succeeds() {
        Scenario::new().run().await.unwrap();
    }

    #[tokio::test]
    async fn before_step_runs_before_each_step() {
        // Hook records the step name before each step; the step records itself.
        // The expected order is: before:A, step:A, before:B, step:B.
        let result = Scenario::new()
            .hook(Hook::before_step("record-before", |ctx, info| {
                let label = format!("before:{}", info.name);
                Box::pin(async move {
                    record_step(ctx, &label);
                    Ok(())
                })
            }))
            .step(Step::named("A").run(|ctx| {
                Box::pin(async move {
                    record_step(ctx, "step:A");
                    Ok(())
                })
            }))
            .step(Step::named("B").run(|ctx| {
                Box::pin(async move {
                    record_step(ctx, "step:B");
                    Ok(())
                })
            }))
            .step(Step::named("collect").run(|ctx| {
                Box::pin(async move {
                    let events = ctx.get(EVENTS)?.clone();
                    // The collect step's own before-hook has also run.
                    assert_eq!(
                        events,
                        vec!["before:A", "step:A", "before:B", "step:B", "before:collect"]
                    );
                    Ok(())
                })
            }))
            .run()
            .await;

        result.unwrap();
    }

    #[tokio::test]
    async fn after_step_runs_after_each_step() {
        let result = Scenario::new()
            .hook(Hook::after_step("record-after", |ctx, info| {
                let label = format!("after:{}", info.name);
                Box::pin(async move {
                    record_step(ctx, &label);
                    Ok(())
                })
            }))
            .step(Step::named("A").run(|ctx| {
                Box::pin(async move {
                    record_step(ctx, "step:A");
                    Ok(())
                })
            }))
            .step(Step::named("B").run(|ctx| {
                Box::pin(async move {
                    record_step(ctx, "step:B");
                    Ok(())
                })
            }))
            .step(Step::named("collect").run(|ctx| {
                Box::pin(async move {
                    let events = ctx.get(EVENTS)?.clone();
                    // Note: 'collect' step itself runs, but we snapshot before its own after-hook.
                    assert_eq!(
                        events,
                        vec!["step:A", "after:A", "step:B", "after:B"]
                    );
                    Ok(())
                })
            }))
            .run()
            .await;

        result.unwrap();
    }

    #[tokio::test]
    async fn hooks_run_in_insertion_order() {
        let result = Scenario::new()
            .hook(Hook::before_step("b1", |ctx, _info| {
                Box::pin(async move {
                    record_step(ctx, "b1");
                    Ok(())
                })
            }))
            .hook(Hook::before_step("b2", |ctx, _info| {
                Box::pin(async move {
                    record_step(ctx, "b2");
                    Ok(())
                })
            }))
            .hook(Hook::after_step("a1", |ctx, _info| {
                Box::pin(async move {
                    record_step(ctx, "a1");
                    Ok(())
                })
            }))
            .hook(Hook::after_step("a2", |ctx, _info| {
                Box::pin(async move {
                    record_step(ctx, "a2");
                    Ok(())
                })
            }))
            .step(Step::named("only").run(|ctx| {
                Box::pin(async move {
                    record_step(ctx, "step");
                    Ok(())
                })
            }))
            .step(Step::named("assert").run(|ctx| {
                Box::pin(async move {
                    let events = ctx.get(EVENTS)?.clone();
                    // First step: b1, b2, step, a1, a2. Second step only runs
                    // its before-hooks before the assertion reads EVENTS.
                    assert_eq!(
                        events,
                        vec!["b1", "b2", "step", "a1", "a2", "b1", "b2"]
                    );
                    Ok(())
                })
            }))
            .run()
            .await;

        result.unwrap();
    }

    #[tokio::test]
    async fn stops_when_before_step_hook_fails() {
        let result = Scenario::new()
            .hook(Hook::before_step("boom", |_ctx, _info| {
                Box::pin(async move { anyhow::bail!("before failure") })
            }))
            .step(Step::named("should_not_run").run(|ctx| {
                Box::pin(async move {
                    record_step(ctx, "step-ran");
                    Ok(())
                })
            }))
            .run()
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("before failure"));
    }

    #[tokio::test]
    async fn stops_when_after_step_hook_fails() {
        let result = Scenario::new()
            .hook(Hook::after_step("boom", |_ctx, _info| {
                Box::pin(async move { anyhow::bail!("after failure") })
            }))
            .step(Step::named("first").run(|_ctx| Box::pin(async move { Ok(()) })))
            .step(Step::named("never_reached").run(|ctx| {
                Box::pin(async move {
                    ctx.insert(COUNTER, 999);
                    Ok(())
                })
            }))
            .run()
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("after failure"));
    }

    #[tokio::test]
    async fn after_step_skipped_when_step_fails() {
        // Use a shared counter on the context: before increments it,
        // after would too. If the step fails, after must NOT run.
        const HITS: Slot<i32> = Slot::new("hits");

        let result = Scenario::new()
            .hook(Hook::before_step("before", |ctx, _info| {
                Box::pin(async move {
                    let v = *ctx.get(HITS).unwrap_or(&0);
                    ctx.insert(HITS, v + 1);
                    Ok(())
                })
            }))
            .hook(Hook::after_step("after", |ctx, _info| {
                Box::pin(async move {
                    let v = *ctx.get(HITS).unwrap_or(&0);
                    ctx.insert(HITS, v + 100);
                    Ok(())
                })
            }))
            .step(Step::named("seed").run(|ctx| {
                Box::pin(async move {
                    ctx.insert(HITS, 0);
                    Ok(())
                })
            }))
            .step(Step::named("fail").run(|_ctx| {
                Box::pin(async move { anyhow::bail!("step failed") })
            }))
            .run()
            .await;

        assert!(result.is_err());
        // We can't inspect ctx after run() consumes the scenario, so we
        // instead verify via a sibling test below using a shared state slot.
        drop(result);
    }

    #[tokio::test]
    async fn after_step_not_called_on_step_failure_observable() {
        // Observable version: use a step before the failing one that snapshots
        // the hit counter on failure isn't possible (run consumes scenario),
        // so we rely on a "witness" step: the after-hook would push an event,
        // but the scenario aborts. A follow-up step that reads the events
        // slot would never run, so we instead count via a static... For a
        // purely in-scenario observable test, use an Arc<AtomicUsize>.
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let before_hits = Arc::new(AtomicUsize::new(0));
        let after_hits = Arc::new(AtomicUsize::new(0));

        let before_hits_clone = before_hits.clone();
        let after_hits_clone = after_hits.clone();

        let result = Scenario::new()
            .hook(Hook::before_step("count-before", move |_ctx, _info| {
                let c = before_hits_clone.clone();
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            }))
            .hook(Hook::after_step("count-after", move |_ctx, _info| {
                let c = after_hits_clone.clone();
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            }))
            .step(Step::named("ok").run(|_ctx| Box::pin(async move { Ok(()) })))
            .step(Step::named("fail").run(|_ctx| {
                Box::pin(async move { anyhow::bail!("step failed") })
            }))
            .run()
            .await;

        assert!(result.is_err());
        // before ran twice (ok, fail), after only ran once (for ok).
        assert_eq!(before_hits.load(Ordering::SeqCst), 2);
        assert_eq!(after_hits.load(Ordering::SeqCst), 1);
    }
}
