use anyhow::Result;

use crate::context::Context;
use crate::step::Step;

/// A sequence of [`Step`]s executed in order against a shared [`Context`].
pub struct Scenario {
    steps: Vec<Step>,
}

impl Scenario {
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    /// Append a step to the scenario (builder pattern).
    pub fn step(mut self, step: Step) -> Self {
        self.steps.push(step);
        self
    }

    /// Execute all steps sequentially. Stops at the first failure.
    pub async fn run(self) -> Result<()> {
        let mut ctx = Context::new();
        for step in &self.steps {
            step.execute(&mut ctx).await?;
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
    use crate::slot::Slot;

    const COUNTER: Slot<i32> = Slot::new("counter");

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
}
