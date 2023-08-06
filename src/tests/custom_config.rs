//! Ensures that a custom config behaves as the default config, until limits are reached.
//! Prevents regression after #80.

#![cfg(not(loom))]

use crate::{Config, Slab};

struct CustomConfig;
impl Config for CustomConfig {
    const INITIAL_PAGE_SIZE: usize = 32;
    const MAX_PAGES: usize = 15;
    const MAX_THREADS: usize = 256;
    const RESERVED_BITS: usize = 24;
}

// We should repeat actions several times to detect invalid lifecycle changes.
const ITERS: u64 = 5;

#[track_caller]
fn slab_eq(mut lhs: Slab<u64, impl Config>, mut rhs: Slab<u64, impl Config>) {
    let mut lhs_vec = lhs.unique_iter().collect::<Vec<_>>();
    lhs_vec.sort_unstable();
    let mut rhs_vec = rhs.unique_iter().collect::<Vec<_>>();
    rhs_vec.sort_unstable();
    assert_eq!(lhs_vec, rhs_vec);
}

/// Calls `insert(); remove()` multiple times to detect invalid releasing.
/// Initially, it revealed bugs in the `Slot::release_with()` implementation.
#[test]
fn insert_remove() {
    let default_slab = Slab::<u64, _>::new();
    let custom_slab = Slab::<u64, _>::new_with_config::<CustomConfig>();

    for i in 0..=ITERS {
        let idx = default_slab.insert(i).unwrap();
        assert!(default_slab.remove(idx));

        let idx = custom_slab.insert(i).unwrap();
        assert!(custom_slab.remove(idx));
    }

    slab_eq(custom_slab, default_slab);
}

/// Calls `get()` multiple times to detect invalid ref counting.
/// Initially, it revealed bugs in the `Slot::get()` implementation.
#[test]
fn double_get() {
    let default_slab = Slab::<u64, _>::new();
    let custom_slab = Slab::<u64, _>::new_with_config::<CustomConfig>();

    for i in 0..=ITERS {
        let idx = default_slab.insert(i).unwrap();
        assert!(default_slab.get(idx).is_some());
        assert!(default_slab.get(idx).is_some());
        assert!(default_slab.remove(idx));

        let idx = custom_slab.insert(i).unwrap();
        assert!(custom_slab.get(idx).is_some());
        assert!(custom_slab.get(idx).is_some());
        assert!(custom_slab.remove(idx));
    }

    slab_eq(custom_slab, default_slab);
}
