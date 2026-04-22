use anyhow::Result;

use crate::context::Context;
use crate::hook::{Hook, HookPoint, StepInfo};
use crate::metrics::{MetricsSink, MetricsStore};
use crate::parallel::Parallel;
use crate::step::Step;

/// A top-level unit of scenario execution.
pub(crate) enum Node {
    Step(Step),
    Parallel(Parallel),
}

/// A sequence of top-level nodes (steps or parallel blocks) executed in
/// order against a shared [`Context`].
pub struct Scenario {
    nodes: Vec<Node>,
    hooks: Vec<Hook>,
}

impl Scenario {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            hooks: Vec::new(),
        }
    }

    /// Append a top-level step (mutable or read-only one-shot).
    pub fn step(mut self, step: Step) -> Self {
        self.nodes.push(Node::Step(step));
        self
    }

    /// Append a parallel block.
    pub fn parallel(mut self, parallel: Parallel) -> Self {
        self.nodes.push(Node::Parallel(parallel));
        self
    }

    /// Attach a lifecycle hook. Hooks run in insertion order.
    pub fn hook(mut self, hook: Hook) -> Self {
        self.hooks.push(hook);
        self
    }

    /// Execute the scenario. The first error stops execution.
    pub async fn run(self) -> Result<()> {
        let (result, _store) = self.run_inner().await;
        result
    }

    /// Test-only hook that returns the collected metrics alongside the
    /// scenario result. Kept crate-private; a public reporting API is
    /// out of scope for this iteration.
    pub(crate) async fn run_inner(self) -> (Result<()>, MetricsStore) {
        let (sink, mut rx) = MetricsSink::new();
        let mut ctx = Context::with_metrics(sink);
        let result = execute_nodes(&self.nodes, &self.hooks, &mut ctx).await;
        drop(ctx);
        let store = MetricsStore::drain_from(&mut rx);
        (result, store)
    }
}

impl Default for Scenario {
    fn default() -> Self {
        Self::new()
    }
}

async fn execute_nodes(nodes: &[Node], hooks: &[Hook], ctx: &mut Context) -> Result<()> {
    for node in nodes {
        match node {
            Node::Step(step) => {
                let info = StepInfo { name: step.name() };
                run_hooks(hooks, HookPoint::BeforeStep, ctx, &info).await?;
                step.execute_top(ctx).await?;
                run_hooks(hooks, HookPoint::AfterStep, ctx, &info).await?;
            }
            Node::Parallel(par) => {
                // Hooks are step-level. Before-hooks for each child run
                // sequentially (with `&mut Context`) before any body starts.
                for step in par.steps() {
                    let info = StepInfo { name: step.name() };
                    run_hooks(hooks, HookPoint::BeforeStep, ctx, &info).await?;
                }

                let block_result = par.execute_bodies(&*ctx).await;

                // After-hooks run only if the whole block succeeded,
                // preserving the rule that after_step runs only after a
                // successful step.
                if block_result.is_ok() {
                    for step in par.steps() {
                        let info = StepInfo { name: step.name() };
                        run_hooks(hooks, HookPoint::AfterStep, ctx, &info).await?;
                    }
                }

                block_result?;
            }
        }
    }
    Ok(())
}

async fn run_hooks(
    hooks: &[Hook],
    point: HookPoint,
    ctx: &mut Context,
    info: &StepInfo<'_>,
) -> Result<()> {
    for hook in hooks {
        if hook.point() == point {
            (hook.func)(ctx, info).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

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

    // ----- Preserved coverage: sequential mutable scenarios -----

    #[tokio::test]
    async fn scenario_runs_steps_in_order() {
        Scenario::new()
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
                    assert_eq!(*ctx.get(COUNTER)?, 1);
                    Ok(())
                })
            }))
            .run()
            .await
            .unwrap();
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

    // ----- Existing hook coverage preserved -----

    #[tokio::test]
    async fn before_step_runs_before_each_step() {
        Scenario::new()
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
                    assert_eq!(
                        events,
                        vec!["before:A", "step:A", "before:B", "step:B", "before:collect"]
                    );
                    Ok(())
                })
            }))
            .run()
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn after_step_runs_after_each_step() {
        Scenario::new()
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
                    assert_eq!(events, vec!["step:A", "after:A", "step:B", "after:B"]);
                    Ok(())
                })
            }))
            .run()
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn hooks_run_in_insertion_order() {
        Scenario::new()
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
                    assert_eq!(
                        events,
                        vec!["b1", "b2", "step", "a1", "a2", "b1", "b2"]
                    );
                    Ok(())
                })
            }))
            .run()
            .await
            .unwrap();
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
        assert!(result.unwrap_err().to_string().contains("after failure"));
    }

    #[tokio::test]
    async fn after_step_not_called_on_step_failure() {
        let before_hits = Arc::new(AtomicUsize::new(0));
        let after_hits = Arc::new(AtomicUsize::new(0));
        let b = before_hits.clone();
        let a = after_hits.clone();

        let result = Scenario::new()
            .hook(Hook::before_step("count-before", move |_ctx, _info| {
                let c = b.clone();
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            }))
            .hook(Hook::after_step("count-after", move |_ctx, _info| {
                let c = a.clone();
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
        assert_eq!(before_hits.load(Ordering::SeqCst), 2);
        assert_eq!(after_hits.load(Ordering::SeqCst), 1);
    }

    // ----- New: parallel + concurrent + metrics -----

    #[tokio::test]
    async fn parallel_runs_children_concurrently_and_waits_for_all() {
        let hits = Arc::new(AtomicUsize::new(0));
        let h_a = hits.clone();
        let h_b = hits.clone();
        let h_c = hits.clone();

        let start = Instant::now();
        Scenario::new()
            .parallel(
                Parallel::named("load")
                    .step(Step::named("a").run_readonly(move |_ctx| {
                        let h = h_a.clone();
                        Box::pin(async move {
                            tokio::time::sleep(Duration::from_millis(80)).await;
                            h.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        })
                    }))
                    .step(Step::named("b").run_readonly(move |_ctx| {
                        let h = h_b.clone();
                        Box::pin(async move {
                            tokio::time::sleep(Duration::from_millis(80)).await;
                            h.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        })
                    }))
                    .step(Step::named("c").run_readonly(move |_ctx| {
                        let h = h_c.clone();
                        Box::pin(async move {
                            tokio::time::sleep(Duration::from_millis(80)).await;
                            h.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        })
                    })),
            )
            .run()
            .await
            .unwrap();

        let elapsed = start.elapsed();
        // wait-all: all children ran
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        // concurrent: ~80ms rather than ~240ms
        assert!(
            elapsed < Duration::from_millis(200),
            "parallel elapsed too long: {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn parallel_fails_if_any_child_fails() {
        let result = Scenario::new()
            .parallel(
                Parallel::named("p")
                    .step(Step::named("ok").run_readonly(|_ctx| {
                        Box::pin(async move { Ok(()) })
                    }))
                    .step(Step::named("boom").run_readonly(|_ctx| {
                        Box::pin(async move { anyhow::bail!("child boom") })
                    })),
            )
            .run()
            .await;
        assert!(result.unwrap_err().to_string().contains("child boom"));
    }

    #[tokio::test]
    async fn concurrent_step_runs_n_workers() {
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();

        Scenario::new()
            .parallel(Parallel::named("p").step(
                Step::named("w").workers(4).run_readonly(move |_ctx| {
                    let c = c.clone();
                    Box::pin(async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                }),
            ))
            .run()
            .await
            .unwrap();

        assert_eq!(count.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn concurrent_step_fails_if_any_worker_fails() {
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();

        let result = Scenario::new()
            .parallel(Parallel::named("p").step(
                Step::named("w").workers(4).run_readonly(move |_ctx| {
                    let c = c.clone();
                    Box::pin(async move {
                        let n = c.fetch_add(1, Ordering::SeqCst);
                        if n == 2 {
                            anyhow::bail!("worker {} failed", n);
                        }
                        Ok(())
                    })
                }),
            ))
            .run()
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn hooks_are_step_level_not_worker_level() {
        let before = Arc::new(AtomicUsize::new(0));
        let after = Arc::new(AtomicUsize::new(0));
        let b = before.clone();
        let a = after.clone();

        Scenario::new()
            .hook(Hook::before_step("before", move |_ctx, _info| {
                let c = b.clone();
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            }))
            .hook(Hook::after_step("after", move |_ctx, _info| {
                let c = a.clone();
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            }))
            .parallel(Parallel::named("p").step(
                Step::named("w")
                    .workers(8)
                    .run_readonly(|_ctx| Box::pin(async move { Ok(()) })),
            ))
            .run()
            .await
            .unwrap();

        assert_eq!(before.load(Ordering::SeqCst), 1);
        assert_eq!(after.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn metrics_sink_collects_events_from_workers() {
        let scenario = Scenario::new().parallel(
            Parallel::named("p")
                .step(Step::named("reads").workers(3).run_readonly(|ctx| {
                    Box::pin(async move {
                        ctx.runtime().metrics().counter("read.success", 1);
                        ctx.runtime().metrics().latency(
                            "read.latency",
                            Duration::from_millis(1),
                        );
                        Ok(())
                    })
                }))
                .step(Step::named("monitor").run_readonly(|ctx| {
                    Box::pin(async move {
                        ctx.runtime()
                            .metrics()
                            .event("monitor", "hello");
                        Ok(())
                    })
                })),
        );

        let (result, store) = scenario.run_inner().await;
        result.unwrap();

        assert_eq!(store.counters.get("read.success").copied(), Some(3));
        assert_eq!(
            store.latencies.get("read.latency").map(|v| v.len()),
            Some(3)
        );
        assert!(store.events.iter().any(|e| matches!(
            e,
            crate::metrics::MetricEvent::Event { name, .. } if name == "monitor"
        )));
    }

    #[tokio::test]
    async fn top_level_mutable_step_still_uses_mutable_context() {
        Scenario::new()
            .step(Step::named("seed").run(|ctx| {
                Box::pin(async move {
                    ctx.insert(COUNTER, 7);
                    Ok(())
                })
            }))
            .step(Step::named("check").run(|ctx| {
                Box::pin(async move {
                    assert_eq!(*ctx.get(COUNTER)?, 7);
                    Ok(())
                })
            }))
            .run()
            .await
            .unwrap();
    }
}
