use std::any::{Any, type_name};
use std::collections::HashMap;
use std::fmt;

use crate::metrics::MetricsSink;
use crate::params::Params;
use crate::slot::Slot;

/// Dynamic storage for typed values keyed by [`Slot`].
struct Extensions {
    map: HashMap<&'static str, Box<dyn Any + Send + Sync>>,
}

impl Extensions {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
}

/// Framework-owned runtime capabilities available to steps and hooks.
///
/// Exposes a [`MetricsSink`] and read-only [`Params`]; future runtime
/// features (deadlines, cancellation, etc.) can live here too.
pub struct RuntimeContext {
    metrics: MetricsSink,
    params: Params,
}

impl RuntimeContext {
    pub(crate) fn new(metrics: MetricsSink, params: Params) -> Self {
        Self { metrics, params }
    }

    pub fn metrics(&self) -> &MetricsSink {
        &self.metrics
    }

    pub fn params(&self) -> &Params {
        &self.params
    }
}

/// Central state container passed to every [`Step`](crate::Step).
pub struct Context {
    runtime: RuntimeContext,
    extensions: Extensions,
}

impl Context {
    /// Create a context with a standalone metrics sink whose receiver is
    /// discarded and empty params. Useful for unit tests that exercise
    /// [`Context`] directly.
    pub fn new() -> Self {
        let (sink, _rx) = MetricsSink::new();
        Self {
            runtime: RuntimeContext::new(sink, Params::empty()),
            extensions: Extensions::new(),
        }
    }

    pub(crate) fn with_runtime(metrics: MetricsSink, params: Params) -> Self {
        Self {
            runtime: RuntimeContext::new(metrics, params),
            extensions: Extensions::new(),
        }
    }

    pub fn runtime(&self) -> &RuntimeContext {
        &self.runtime
    }

    /// Insert a value into the context. Returns the previous value if one existed.
    pub fn insert<T: Send + Sync + 'static>(&mut self, slot: Slot<T>, value: T) -> Option<T> {
        self.extensions
            .map
            .insert(slot.name(), Box::new(value))
            .and_then(|old| old.downcast::<T>().ok().map(|b| *b))
    }

    /// Get an immutable reference to a stored value.
    pub fn get<T: Send + Sync + 'static>(&self, slot: Slot<T>) -> Result<&T, ContextError> {
        match self.extensions.map.get(slot.name()) {
            Some(boxed) => boxed
                .downcast_ref::<T>()
                .ok_or_else(|| ContextError::TypeMismatch {
                    slot_name: slot.name(),
                    expected: type_name::<T>(),
                    actual: (**boxed).type_id(),
                }),
            None => Err(ContextError::NotFound {
                slot_name: slot.name(),
                expected: type_name::<T>(),
                registered: self.registered_slots(),
            }),
        }
    }

    /// Get a mutable reference to a stored value.
    pub fn get_mut<T: Send + Sync + 'static>(
        &mut self,
        slot: Slot<T>,
    ) -> Result<&mut T, ContextError> {
        if !self.extensions.map.contains_key(slot.name()) {
            return Err(ContextError::NotFound {
                slot_name: slot.name(),
                expected: type_name::<T>(),
                registered: self.registered_slots(),
            });
        }
        // Check type first via immutable borrow to capture type_id if needed.
        let actual_type_id = (*self.extensions.map[slot.name()]).type_id();
        let boxed = self.extensions.map.get_mut(slot.name()).unwrap();
        boxed.downcast_mut::<T>().ok_or(ContextError::TypeMismatch {
            slot_name: slot.name(),
            expected: type_name::<T>(),
            actual: actual_type_id,
        })
    }

    /// Remove a value from the context and return it.
    pub fn remove<T: Send + Sync + 'static>(&mut self, slot: Slot<T>) -> Result<T, ContextError> {
        let registered = self.registered_slots();
        match self.extensions.map.remove(slot.name()) {
            Some(boxed) => boxed.downcast::<T>().map(|b| *b).map_err(|boxed| {
                // Put it back since downcast failed
                self.extensions.map.insert(slot.name(), boxed);
                ContextError::TypeMismatch {
                    slot_name: slot.name(),
                    expected: type_name::<T>(),
                    actual: (*self.extensions.map[slot.name()]).type_id(),
                }
            }),
            None => Err(ContextError::NotFound {
                slot_name: slot.name(),
                expected: type_name::<T>(),
                registered,
            }),
        }
    }

    /// Check whether a slot is present in the context.
    pub fn contains<T>(&self, slot: Slot<T>) -> bool {
        self.extensions.map.contains_key(slot.name())
    }

    fn registered_slots(&self) -> Vec<&'static str> {
        self.extensions.map.keys().copied().collect()
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can occur when accessing a [`Context`].
#[derive(Debug)]
pub enum ContextError {
    /// The requested slot was not found.
    NotFound {
        slot_name: &'static str,
        expected: &'static str,
        registered: Vec<&'static str>,
    },
    /// The slot exists but contains a value of a different type.
    TypeMismatch {
        slot_name: &'static str,
        expected: &'static str,
        actual: std::any::TypeId,
    },
}

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextError::NotFound {
                slot_name,
                expected,
                registered,
            } => {
                write!(
                    f,
                    "slot '{}' not found (expected type: {}). Registered slots: [{}]",
                    slot_name,
                    expected,
                    registered.join(", ")
                )
            }
            ContextError::TypeMismatch {
                slot_name,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "slot '{}' type mismatch: expected {}, got {:?}",
                    slot_name, expected, actual
                )
            }
        }
    }
}

impl std::error::Error for ContextError {}

#[cfg(test)]
mod tests {
    use super::*;

    const COUNT: Slot<u32> = Slot::new("count");
    const NAME: Slot<String> = Slot::new("name");

    #[test]
    fn insert_and_get() {
        let mut ctx = Context::new();
        assert!(!ctx.contains(COUNT));

        ctx.insert(COUNT, 42);
        assert!(ctx.contains(COUNT));
        assert_eq!(*ctx.get(COUNT).unwrap(), 42);
    }

    #[test]
    fn insert_overwrites() {
        let mut ctx = Context::new();
        assert!(ctx.insert(COUNT, 1).is_none());
        assert_eq!(ctx.insert(COUNT, 2), Some(1));
        assert_eq!(*ctx.get(COUNT).unwrap(), 2);
    }

    #[test]
    fn get_not_found() {
        let ctx = Context::new();
        let err = ctx.get(COUNT).unwrap_err();
        assert!(matches!(err, ContextError::NotFound { .. }));
        let msg = err.to_string();
        assert!(msg.contains("count"));
    }

    #[test]
    fn get_mut_works() {
        let mut ctx = Context::new();
        ctx.insert(COUNT, 10);
        *ctx.get_mut(COUNT).unwrap() += 5;
        assert_eq!(*ctx.get(COUNT).unwrap(), 15);
    }

    #[test]
    fn remove_works() {
        let mut ctx = Context::new();
        ctx.insert(NAME, "hello".to_string());
        let val = ctx.remove(NAME).unwrap();
        assert_eq!(val, "hello");
        assert!(!ctx.contains(NAME));
    }

    #[test]
    fn remove_not_found() {
        let mut ctx = Context::new();
        let err = ctx.remove(NAME).unwrap_err();
        assert!(matches!(err, ContextError::NotFound { .. }));
    }

    #[test]
    fn error_shows_registered_slots() {
        let mut ctx = Context::new();
        ctx.insert(COUNT, 1);
        ctx.insert(NAME, "x".to_string());

        let err = ctx.get(Slot::<bool>::new("missing")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("count") || msg.contains("name"));
    }
}
