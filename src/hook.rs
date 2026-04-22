use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

use crate::context::Context;

/// Points in a step's lifecycle at which a [`Hook`] can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookPoint {
    BeforeStep,
    AfterStep,
}

/// Lightweight metadata about the step surrounding a hook invocation.
pub struct StepInfo<'a> {
    pub name: &'a str,
}

/// The function type stored inside a [`Hook`].
pub type HookFn = Box<
    dyn for<'a> Fn(
            &'a mut Context,
            &'a StepInfo<'a>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>
        + Send
        + Sync,
>;

/// A named, async callback that runs before or after a [`Step`](crate::Step).
///
/// Hooks are intended for cross-cutting observation / instrumentation
/// (logging, timing, tracing). Workload logic belongs in a `Step`.
pub struct Hook {
    name: String,
    point: HookPoint,
    pub(crate) func: HookFn,
}

impl Hook {
    /// Create a hook that runs before each step.
    ///
    /// # Example
    /// ```ignore
    /// Hook::before_step("log-start", |_ctx, info| Box::pin(async move {
    ///     println!("starting: {}", info.name);
    ///     Ok(())
    /// }))
    /// ```
    pub fn before_step<F>(name: impl Into<String>, f: F) -> Self
    where
        F: for<'a> Fn(
                &'a mut Context,
                &'a StepInfo<'a>,
            ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>
            + Send
            + Sync
            + 'static,
    {
        Self {
            name: name.into(),
            point: HookPoint::BeforeStep,
            func: Box::new(f),
        }
    }

    /// Create a hook that runs after each successful step.
    pub fn after_step<F>(name: impl Into<String>, f: F) -> Self
    where
        F: for<'a> Fn(
                &'a mut Context,
                &'a StepInfo<'a>,
            ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>
            + Send
            + Sync
            + 'static,
    {
        Self {
            name: name.into(),
            point: HookPoint::AfterStep,
            func: Box::new(f),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn point(&self) -> HookPoint {
        self.point
    }
}

/// Wraps an async function into a hook-compatible closure.
///
/// # Example
/// ```ignore
/// async fn log_start(_ctx: &mut Context, info: &StepInfo<'_>) -> Result<()> {
///     println!("starting: {}", info.name);
///     Ok(())
/// }
///
/// Hook::before_step("log-start", hook_fn!(log_start))
/// ```
#[macro_export]
macro_rules! hook_fn {
    ($f:expr) => {
        |ctx, info| Box::pin($f(ctx, info))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hook_stores_name_and_point() {
        let h = Hook::before_step("h1", |_ctx, _info| Box::pin(async { Ok(()) }));
        assert_eq!(h.name(), "h1");
        assert_eq!(h.point(), HookPoint::BeforeStep);

        let h = Hook::after_step("h2", |_ctx, _info| Box::pin(async { Ok(()) }));
        assert_eq!(h.name(), "h2");
        assert_eq!(h.point(), HookPoint::AfterStep);
    }
}
