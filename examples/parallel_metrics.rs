use std::time::Duration;

use anyhow::Result;
use undine::{Context, Hook, Parallel, Scenario, Step, StepInfo, hook_fn, step_fn};

async fn setup(_ctx: &mut Context) -> Result<()> {
    println!("-- setup");
    Ok(())
}

async fn read_worker(ctx: &Context) -> Result<()> {
    let start = std::time::Instant::now();
    tokio::time::sleep(Duration::from_millis(10)).await;
    ctx.runtime()
        .metrics()
        .latency("read.latency", start.elapsed());
    ctx.runtime().metrics().counter("read.success", 1);
    Ok(())
}

async fn write_worker(ctx: &Context) -> Result<()> {
    let start = std::time::Instant::now();
    tokio::time::sleep(Duration::from_millis(15)).await;
    ctx.runtime()
        .metrics()
        .latency("write.latency", start.elapsed());
    ctx.runtime().metrics().counter("write.success", 1);
    Ok(())
}

async fn monitor(ctx: &Context) -> Result<()> {
    ctx.runtime()
        .metrics()
        .event("monitor", "monitor step executed");
    Ok(())
}

async fn validate(_ctx: &mut Context) -> Result<()> {
    println!("-- validate");
    Ok(())
}

async fn log_start(_ctx: &mut Context, info: &StepInfo<'_>) -> Result<()> {
    println!("[before] {}", info.name);
    Ok(())
}

async fn log_end(_ctx: &mut Context, info: &StepInfo<'_>) -> Result<()> {
    println!("[after ] {}", info.name);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let summary = Scenario::new()
        .hook(Hook::before_step("log-start", hook_fn!(log_start)))
        .hook(Hook::after_step("log-end", hook_fn!(log_end)))
        .step(Step::named("setup").run(step_fn!(setup)))
        .parallel(
            Parallel::named("load")
                .step(
                    Step::named("read")
                        .workers(4)
                        .duration(Duration::from_millis(50))
                        .run_readonly(step_fn!(read_worker)),
                )
                .step(
                    Step::named("write")
                        .workers(2)
                        .duration(Duration::from_millis(50))
                        .run_readonly(step_fn!(write_worker)),
                )
                .step(Step::named("monitor").run_readonly(step_fn!(monitor))),
        )
        .step(Step::named("validate").run(step_fn!(validate)))
        .run()
        .await?;

    println!("\n--- Run summary ---");
    println!("{summary:#?}");
    Ok(())
}
