use anyhow::Result;
use undine::{Context, Scenario, Slot, Step, step_fn};

const MESSAGE: Slot<String> = Slot::new("message");
const COUNT: Slot<u32> = Slot::new("count");

async fn setup(ctx: &mut Context) -> Result<()> {
    ctx.insert(MESSAGE, "Hello, Undine!".to_string());
    ctx.insert(COUNT, 0);
    Ok(())
}

async fn run(ctx: &mut Context) -> Result<()> {
    let msg = ctx.get(MESSAGE)?;
    println!("{msg}");

    let count = ctx.get_mut(COUNT)?;
    *count += 1;
    println!("count: {count}");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    Scenario::new()
        .step(Step::named("setup").run(step_fn!(setup)))
        .step(Step::named("run").run(step_fn!(run)))
        .run()
        .await
}
