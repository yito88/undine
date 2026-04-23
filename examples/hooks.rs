use anyhow::Result;
use undine::{Context, Hook, Scenario, Slot, Step, StepInfo, hook_fn, step_fn};

const MESSAGE: Slot<String> = Slot::new("message");
const COUNT: Slot<u32> = Slot::new("count");

async fn setup(ctx: &mut Context) -> Result<()> {
    ctx.insert(MESSAGE, "Hello, Undine!".to_string());
    ctx.insert(COUNT, 0);
    Ok(())
}

async fn load(ctx: &mut Context) -> Result<()> {
    let msg = ctx.get(MESSAGE)?;
    println!("  -> {msg}");
    let count = ctx.get_mut(COUNT)?;
    *count += 1;
    Ok(())
}

async fn before_step_log(_ctx: &mut Context, info: &StepInfo<'_>) -> Result<()> {
    println!("[before] {}", info.name);
    Ok(())
}

async fn after_step_log(_ctx: &mut Context, info: &StepInfo<'_>) -> Result<()> {
    println!("[after ] {}", info.name);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    Scenario::new()
        .hook(Hook::before_step("log-start", hook_fn!(before_step_log)))
        .hook(Hook::after_step("log-end", hook_fn!(after_step_log)))
        .step(Step::named("setup").run(step_fn!(setup)))
        .step(Step::named("load").run(step_fn!(load)))
        .run()
        .await?;
    Ok(())
}
