use std::time::Duration;

use anyhow::Result;
use undine::{Context, Parallel, Params, Scenario, Step, step_fn};

const TOML: &str = r#"
[params]
workers = 4
duration_ms = 30
read_ratio = 80
injection_mode = "none"
"#;

async fn setup(ctx: &mut Context) -> Result<()> {
    let workers = ctx.runtime().params().get_usize("workers")?;
    let mode = ctx.runtime().params().get_str("injection_mode")?;
    println!("setup: workers={workers}, mode={mode}");
    Ok(())
}

async fn read_worker(ctx: &Context) -> Result<()> {
    let ratio = ctx.runtime().params().get_u64("read_ratio")?;
    ctx.runtime().metrics().counter("read.ratio_seen", ratio);
    let start = std::time::Instant::now();
    tokio::time::sleep(Duration::from_millis(5)).await;
    ctx.runtime()
        .metrics()
        .latency("read.latency", start.elapsed());
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let params = Params::from_toml_str(TOML)?;
    let workers = params.get_usize("workers")?;
    let duration_ms = params.get_u64("duration_ms")?;

    let summary = Scenario::new()
        .step(Step::named("setup").run(step_fn!(setup)))
        .parallel(
            Parallel::named("load").step(
                Step::named("read")
                    .workers(workers)
                    .duration(Duration::from_millis(duration_ms))
                    .run_readonly(step_fn!(read_worker)),
            ),
        )
        .run_with_params(params)
        .await?;

    println!("\n--- Run summary ---");
    println!("{summary:#?}");
    Ok(())
}
