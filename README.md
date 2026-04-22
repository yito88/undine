# Undine

Undine is a Rust-native scenario framework for writing benchmarks and failure tests as code.

The core idea is:

- Use **top-level steps** for orchestration (`setup`, `validate`, `cleanup`)
- Use **parallel blocks** when different roles should run at the same time
- Use **concurrent steps** (`workers(n)`) when one logical workload should run with multiple workers
- Keep cross-cutting behavior in **hooks**
- Keep runtime observations in **metrics**

This README focuses on **how to write Undine tests** with the current API.

## Summary

Undine is currently best thought of as:

- **Scenario** for phase orchestration
- **Step** for named actions
- **Parallel** for heterogeneous concurrency
- **workers(n)** for homogeneous concurrency
- **Hook** for step-level observation
- **MetricsSink** for runtime measurements from read-only workers

If you are writing benchmarks or failure tests today, the most natural shape is:

- Mutable top-level setup/validate steps
- Read-only concurrent workers inside a parallel load phase
- Request-level measurements emitted directly from worker code

## Mental model

A typical Undine scenario looks like this:

```rust
Scenario::new()
    .step(Step::named("setup").run(step_fn!(setup)))
    .parallel(
        Parallel::named("load")
            .step(
                Step::named("read")
                    .workers(4)
                    .duration(Duration::from_secs(60))
                    .run_readonly(step_fn!(read_worker)),
            )
            .step(
                Step::named("write")
                    .workers(4)
                    .duration(Duration::from_secs(60))
                    .run_readonly(step_fn!(write_worker)),
            )
            .step(
                Step::named("monitor")
                    .run_readonly(step_fn!(monitor)),
            ),
    )
    .step(Step::named("validate").run(step_fn!(validate)))
    .run()
    .await?;
```

Read that as:

1. Run `setup`
2. Run `read`, `write`, and `monitor` together
3. Run `validate`

`Parallel` is for **different roles at the same time**. `workers(n)` is for **the same role with N workers**. This is how the current implementation is structured. `Scenario` stores top-level nodes as either `Step` or `Parallel`, and `Parallel` executes child step bodies concurrently with wait-all semantics.

---

## 1. Start with a basic sequential test

If your test is just setup → run → validate, use top-level mutable steps.

```rust
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
```

This is the same pattern as the `basic` example: it uses typed slots, mutates `Context`, and runs steps in order.

### When to use this style

Use top-level mutable steps for orchestration-style phases:

- Setup
- Seed data
- Create handles/resources
- Validation
- Cleanup

In the current implementation, `Step::run(...)` is the mutable top-level form, while `Step::run_readonly(...)` is the read-only form used for parallel/concurrent work. `workers > 1` and `duration(...)` are only meaningful with `run_readonly(...)`. If you set `workers(...)` on a mutable step, execution fails.

---

## 2. Share state with typed slots

Undine uses a dynamic context with typed named slots.

```rust
const CLIENT: Slot<MyClient> = Slot::new("client");
const RUN_ID: Slot<String> = Slot::new("run_id");
```

Then store and retrieve values from `Context`:

```rust
ctx.insert(CLIENT, client);
let client = ctx.get(CLIENT)?;
```

`Slot<T>` is a typed key backed by a static name. `Context` stores values dynamically and provides `insert`, `get`, `get_mut`, `remove`, and `contains`. Missing slots and type mismatches return structured errors.

### Recommended use

Put these in `Context`:

- Clients
- Handles
- Resources needed by top-level orchestration steps

Examples:

- DB clients
- Cluster handles
- Process handles
- Test inputs created in setup and consumed later

Use `Context` less as a global metrics store, and more as **scenario state needed to run the test**.

---

## 3. Add hooks for step-level observation

Undine currently supports two hook points:

- `before_step`
- `after_step`

A basic hook example:

```rust
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
    println!(" -> {msg}");

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
        .await
}
```

This matches the `hooks` example.

### What hooks are good for

Use hooks for cross-cutting observation and instrumentation:

- Step start/end logs
- Coarse timing
- Tracing
- Metadata emission

Do **not** put core workload logic in hooks.

### Important current behavior in `Parallel`

Hooks are still **step-level**, even inside a `Parallel` block:

- `before_step` for each child runs sequentially before any child body starts
- `after_step` for child steps runs only if the whole block succeeds

That means `after_step` is **not** a per-request or per-worker measurement hook.

---

## 4. Use `Parallel` for different roles running together

When different logical roles should run at the same time, use `Parallel`.

Example roles:

- Read workload
- Write workload
- Monitor
- Fault-driving client

`Parallel` is a named block of child steps. The current implementation runs all child bodies concurrently and waits for all of them to finish. If any child step fails, the block fails. This is **wait-all**, not fail-fast.

Minimal shape:

```rust
Scenario::new()
    .step(Step::named("setup").run(step_fn!(setup)))
    .parallel(
        Parallel::named("load")
            .step(Step::named("read").run_readonly(step_fn!(read_worker)))
            .step(Step::named("write").run_readonly(step_fn!(write_worker)))
            .step(Step::named("monitor").run_readonly(step_fn!(monitor))),
    )
    .step(Step::named("validate").run(step_fn!(validate)))
    .run()
    .await?;
```

### Rule of thumb

Inside `Parallel`, use `run_readonly(...)`.

That is not just style. The current implementation requires child steps inside `Parallel` to use the read-only runner. A mutable child step errors at runtime.

---

## 5. Use `workers(n)` + `duration(...)` for concurrent load

This is the main benchmark/failure-test pattern.

A concurrent step is still one logical step, but its body is executed by multiple workers:

```rust
Step::named("read")
    .workers(4)
    .duration(Duration::from_secs(60))
    .run_readonly(step_fn!(read_worker))
```

Current semantics:

- Default `workers = 1`
- Default `duration = None`
- With `workers > 1`, the read-only body runs concurrently N times
- With `duration(d)`, each worker loops the body until the deadline is reached

This makes `workers(...)` good for homogeneous concurrency:

- 4 read workers
- 8 write workers
- 16 polling workers

### Worker function shape

Workers receive `&Context`, not `&mut Context`:

```rust
async fn read_worker(ctx: &Context) -> Result<()> {
    // perform one operation
    Ok(())
}
```

That is intentional. In the current design, parallel/concurrent work is modeled as **clients acting on the test target**, not as mutators of shared scenario state.

---

## 6. Record request-level metrics from the worker itself

For fine-grained benchmark data, measure inside the worker.

```rust
async fn read_worker(ctx: &Context) -> Result<()> {
    let start = std::time::Instant::now();

    // do one request
    tokio::time::sleep(Duration::from_millis(10)).await;

    ctx.runtime().metrics().latency("read.latency", start.elapsed());
    ctx.runtime().metrics().counter("read.success", 1);

    Ok(())
}
```

The `parallel_metrics` example does exactly this for read and write workers, and also emits an event from `monitor`.

### Why not use `after_step` for latency?

Because `after_step` is too coarse.

For a step like:

```rust
Step::named("read")
    .workers(4)
    .duration(Duration::from_secs(60))
    .run_readonly(step_fn!(read_worker))
```

`after_step` only runs once after the **entire logical step** completes. It does not see individual requests, and it does not run per worker. Current hooks are step-level only.

So the recommended split is:

- **Worker code**: per-request latency, success/failure counters, operation events
- **Hooks**: step lifecycle logging and coarse observation

---

## 7. Built-in metrics are available from `RuntimeContext`

Undine currently provides a built-in `MetricsSink` through `RuntimeContext`:

```rust
ctx.runtime().metrics().counter("read.success", 1);
ctx.runtime().metrics().latency("read.latency", elapsed);
ctx.runtime().metrics().event("monitor", "monitor step executed");
```

`MetricEvent` currently has three variants:

- `Counter`
- `Latency`
- `Event`

This sink is channel-based and exists so read-only workers can emit measurements safely while only receiving `&Context`. `RuntimeContext` currently exposes `metrics()` as its main capability.

### Current limitation

The crate already collects these metrics internally during scenario execution, but **there is no public reporting API yet**. `Scenario::run()` returns only `Result<()>`, while the metrics store remains internal.

That means metrics emission is available now, but consuming the aggregated result is still a future API design area.

---

## 8. Full example

This is the current “real” Undine shape:

```rust
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
    ctx.runtime().metrics().latency("read.latency", start.elapsed());
    ctx.runtime().metrics().counter("read.success", 1);
    Ok(())
}

async fn write_worker(ctx: &Context) -> Result<()> {
    let start = std::time::Instant::now();
    tokio::time::sleep(Duration::from_millis(15)).await;
    ctx.runtime().metrics().latency("write.latency", start.elapsed());
    ctx.runtime().metrics().counter("write.success", 1);
    Ok(())
}

async fn monitor(ctx: &Context) -> Result<()> {
    ctx.runtime().metrics().event("monitor", "monitor step executed");
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
    Scenario::new()
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
                .step(
                    Step::named("monitor")
                        .run_readonly(step_fn!(monitor)),
                ),
        )
        .step(Step::named("validate").run(step_fn!(validate)))
        .run()
        .await
}
```

This is essentially the `parallel_metrics` example.

---

## 9. Recommended style for writing Undine tests

A practical recipe:

### Use top-level mutable steps for orchestration
Good:

- Setup
- Prepare a cluster
- Validate
- Cleanup

### Use `Parallel` for mixed roles
Good:

- Read
- Write
- Monitor
- Injector

### Use `workers(n)` for scale
Good:

- `read workers x 32`
- `write workers x 16`

### Emit request-level metrics inside workers
Good:

- Latency
- Success/Failure counters
- Operation markers

### Use hooks only for step-level observation
Good:

- Start/End logs
- Coarse timing
- Tracing markers

---

## 10. Current limitations to keep in mind

Today’s implementation is intentionally small. A few important current constraints:

- Hooks are only `before_step` and `after_step`
- Hooks are step-level, not worker-level
- Child steps inside `Parallel` must use `run_readonly(...)`
- Metrics can be emitted, but there is no public metrics/report API yet
- `Parallel` is currently one level deep; nested parallel composition is not documented as supported in the current README draft
- Cancellation, timeout guards for `Parallel`, richer reporting, and worker-level hooks are future extensions rather than current public surface

---

## 11. Running the examples

```bash
cargo run --example basic
cargo run --example hooks
cargo run --example parallel_metrics
```

Examples are declared in `Cargo.toml`.

---

## 12. API cheat sheet

### Mutable top-level step

```rust
async fn setup(ctx: &mut Context) -> Result<()> { ... }

Step::named("setup").run(step_fn!(setup))
```

### Read-only child / worker step

```rust
async fn read_worker(ctx: &Context) -> Result<()> { ... }

Step::named("read").run_readonly(step_fn!(read_worker))
```

### Concurrent worker step

```rust
Step::named("read")
    .workers(4)
    .duration(Duration::from_secs(60))
    .run_readonly(step_fn!(read_worker))
```

### Hook

```rust
async fn log_start(_ctx: &mut Context, info: &StepInfo<'_>) -> Result<()> { ... }

Hook::before_step("log-start", hook_fn!(log_start))
```

### Metrics

```rust
ctx.runtime().metrics().counter("read.success", 1);
ctx.runtime().metrics().latency("read.latency", elapsed);
ctx.runtime().metrics().event("phase", "load started");
```
