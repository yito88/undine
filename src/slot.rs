use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

/// A typed key for storing and retrieving values in a [`Context`](crate::Context).
#[derive(Debug)]
pub struct Slot<T> {
    name: &'static str,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Slot<T> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _marker: PhantomData,
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
}

impl<T> Copy for Slot<T> {}

impl<T> Clone for Slot<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> PartialEq for Slot<T> {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl<T> Eq for Slot<T> {}

impl<T> Hash for Slot<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_copy_clone() {
        let slot: Slot<i32> = Slot::new("x");
        let copied = slot;
        let cloned = slot;
        assert_eq!(slot, copied);
        assert_eq!(slot, cloned);
    }

    #[test]
    fn slot_eq() {
        let a: Slot<i32> = Slot::new("a");
        let b: Slot<i32> = Slot::new("a");
        let c: Slot<i32> = Slot::new("c");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn slot_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Slot::<i32>::new("x"));
        assert!(set.contains(&Slot::<i32>::new("x")));
    }
}
