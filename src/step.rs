use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

use crate::context::Context;

/// The function type stored inside a [`Step`].
pub type StepFn = Box<
    dyn for<'a> Fn(&'a mut Context) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>
        + Send
        + Sync,
>;

/// A named, async action that operates on a [`Context`].
pub struct Step {
    name: String,
    func: Option<StepFn>,
}

impl Step {
    /// Create a new step with the given name.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            func: None,
        }
    }

    /// Attach the async function to execute.
    ///
    /// The function must return a pinned, boxed future. Use the [`step_fn!`](crate::step_fn)
    /// macro for ergonomic wrapping of async functions.
    ///
    /// # Example
    /// ```ignore
    /// Step::named("setup").run(|ctx| Box::pin(async move {
    ///     ctx.insert(SLOT, 42);
    ///     Ok(())
    /// }))
    /// ```
    pub fn run(
        mut self,
        f: impl for<'a> Fn(&'a mut Context) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.func = Some(Box::new(f));
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) async fn execute(&self, ctx: &mut Context) -> Result<()> {
        match &self.func {
            Some(f) => f(ctx).await,
            None => anyhow::bail!("step '{}' has no function attached", self.name),
        }
    }
}

/// Wraps an async function into a step-compatible closure.
///
/// # Example
/// ```ignore
/// async fn setup(ctx: &mut Context) -> Result<()> {
///     ctx.insert(SLOT, 42);
///     Ok(())
/// }
///
/// Step::named("setup").run(step_fn!(setup))
/// ```
#[macro_export]
macro_rules! step_fn {
    ($f:expr) => {
        |ctx| Box::pin($f(ctx))
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slot::Slot;

    const VALUE: Slot<i32> = Slot::new("value");

    #[tokio::test]
    async fn step_executes() {
        let step = Step::named("test").run(|ctx| {
            Box::pin(async move {
                ctx.insert(VALUE, 42);
                Ok(())
            })
        });

        assert_eq!(step.name(), "test");

        let mut ctx = Context::new();
        step.execute(&mut ctx).await.unwrap();
        assert_eq!(*ctx.get(VALUE).unwrap(), 42);
    }

    #[tokio::test]
    async fn step_with_named_fn() {
        async fn my_step(ctx: &mut Context) -> Result<()> {
            ctx.insert(VALUE, 99);
            Ok(())
        }

        let step = Step::named("named").run(step_fn!(my_step));
        let mut ctx = Context::new();
        step.execute(&mut ctx).await.unwrap();
        assert_eq!(*ctx.get(VALUE).unwrap(), 99);
    }

    #[tokio::test]
    async fn step_without_func_errors() {
        let step = Step::named("empty");
        let mut ctx = Context::new();
        let result = step.execute(&mut ctx).await;
        assert!(result.is_err());
    }
}
