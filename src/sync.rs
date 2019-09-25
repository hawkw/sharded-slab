pub(crate) use self::inner::*;

#[cfg(test)]
mod inner {
    pub(crate) use loom::sync::{CausalCell};
    pub(crate) mod atomic {
        pub use loom::sync::atomic::*;
        pub use std::sync::atomic::Ordering;
    }
}

#[cfg(not(test))]
mod inner {
    use std::cell::UnsafeCell;
    pub(crate) use std::sync::{atomic};

    #[derive(Debug)]
    pub struct CausalCell<T>(UnsafeCell<T>);

    impl<T> CausalCell<T> {
        pub fn new(data: T) -> CausalCell<T> {
            CausalCell(UnsafeCell::new(data))
        }

        pub fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(*const T) -> R,
        {
            f(self.0.get())
        }

        pub fn with_mut<F, R>(&self, f: F) -> R
        where
            F: FnOnce(*mut T) -> R,
        {
            f(self.0.get())
        }
    }
}
