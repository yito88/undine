use anyhow::Result;
use undine::{Context, RunSuite, Scenario, Step, step_fn};

const TOML: &str = r#"
[[runs]]
name = "baseline"
mode = "steady"
target = 100

[[runs]]
name = "high-load"
mode = "steady"
target = 500

[[runs]]
name = "burst"
mode = "spike"
target = 250
"#;

async fn setup(ctx: &mut Context) -> Result<()> {
    let mode = ctx.runtime().params().get_str("mode")?;
    let target = ctx.runtime().params().get_u64("target")?;
    println!("-> run starting: mode={mode}, target={target}");
    Ok(())
}

async fn work(ctx: &Context) -> Result<()> {
    let target = ctx.runtime().params().get_u64("target")?;
    ctx.runtime().metrics().counter("target_seen", target);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let suite = RunSuite::from_toml_str(TOML)?;

    let results = Scenario::new()
        .step(Step::named("setup").run(step_fn!(setup)))
        .step(Step::named("work").run_readonly(step_fn!(work)))
        .run_suite(suite)
        .await?;

    println!("\n--- Suite results ---");
    for r in &results {
        println!(
            "run={:?} success={} target_seen={:?}",
            r.run_name,
            r.summary.success,
            r.summary.metrics.counters.get("target_seen").copied()
        );
    }
    Ok(())
}
