pub(crate) use self::inner::*;

#[cfg(loom)]
mod inner {
    pub(crate) use loom::lazy_static;
    pub(crate) use loom::sync::Mutex;
    pub(crate) use loom::{alloc, cell::UnsafeCell};
    pub(crate) mod atomic {
        pub use loom::sync::atomic::*;
        pub use std::sync::atomic::Ordering;
    }
    pub(crate) use loom::lazy_static;
    pub(crate) use loom::sync::Mutex;
    pub(crate) use loom::thread::yield_now;
    pub(crate) use loom::thread_local;
}

#[cfg(not(loom))]
mod inner {
    #![allow(dead_code)]
    pub(crate) use lazy_static::lazy_static;
    pub(crate) use std::sync::{atomic, Mutex};
    pub(crate) use std::thread::yield_now;
    pub(crate) use std::thread_local;

    #[derive(Debug)]
    pub(crate) struct UnsafeCell<T>(std::cell::UnsafeCell<T>);

    impl<T> UnsafeCell<T> {
        pub fn new(data: T) -> UnsafeCell<T> {
            UnsafeCell(std::cell::UnsafeCell::new(data))
        }

        #[inline(always)]
        pub fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(*const T) -> R,
        {
            f(self.0.get())
        }

        #[inline(always)]
        pub fn with_mut<F, R>(&self, f: F) -> R
        where
            F: FnOnce(*mut T) -> R,
        {
            f(self.0.get())
        }
    }

    pub(crate) mod alloc {
        /// Track allocations, detecting leaks
        #[derive(Debug)]
        pub struct Track<T> {
            value: T,
        }

        impl<T> Track<T> {
            /// Track a value for leaks
            #[inline(always)]
            pub fn new(value: T) -> Track<T> {
                Track { value }
            }

            /// Get a reference to the value
            #[inline(always)]
            pub fn get_ref(&self) -> &T {
                &self.value
            }

            /// Get a mutable reference to the value
            #[inline(always)]
            pub fn get_mut(&mut self) -> &mut T {
                &mut self.value
            }

            /// Stop tracking the value for leaks
            #[inline(always)]
            pub fn into_inner(self) -> T {
                self.value
            }
        }
    }
}
