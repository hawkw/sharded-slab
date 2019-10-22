use crate::cfg;
use crate::sync::atomic::{spin_loop_hint, AtomicUsize, Ordering};
use std::{fmt, marker::PhantomData};

pub(super) struct TransferStack<C = cfg::DefaultConfig> {
    head: AtomicUsize,
    _cfg: PhantomData<fn(C)>,
}

impl<C: cfg::Config> TransferStack<C> {
    pub(super) fn new() -> Self {
        Self {
            head: AtomicUsize::new(super::Addr::<C>::NULL),
            _cfg: PhantomData,
        }
    }

    pub(super) fn pop_all(&self) -> Option<usize> {
        let val = self.head.swap(super::Addr::<C>::NULL, Ordering::Acquire);
        test_println!("-> pop {:#x}", val);
        if val == super::Addr::<C>::NULL {
            None
        } else {
            Some(val)
        }
    }

    pub(super) fn push(&self, value: usize, before: impl Fn(usize)) {
        let mut next = self.head.load(Ordering::Relaxed);
        loop {
            test_println!("-> next {:#x}", next);
            before(next);

            match self
                .head
                .compare_exchange(next, value, Ordering::Release, Ordering::Relaxed)
            {
                // lost the race!
                Err(actual) => {
                    test_println!("-> retry!");
                    next = actual;
                }
                Ok(_) => {
                    test_println!("-> successful; next={:#x}", next);
                    return;
                }
            }
        }
    }
}

impl<C> fmt::Debug for TransferStack<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransferStack")
            .field(
                "head",
                &format_args!("{:#0x}", &self.head.load(Ordering::Relaxed)),
            )
            .finish()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{sync::CausalCell, test_util};
    use loom::thread;
    use std::sync::Arc;

    #[test]
    fn transfer_stack() {
        test_util::run_model("transfer_stack", || {
            let causalities = [CausalCell::new(999), CausalCell::new(999)];
            let shared = Arc::new((causalities, TransferStack::<cfg::DefaultConfig>::new()));
            let shared1 = shared.clone();
            let shared2 = shared.clone();

            let t1 = thread::spawn(move || {
                let (causalities, stack) = &*shared1;
                stack.push(0, |prev| {
                    causalities[0].with_mut(|c| unsafe {
                        *c = 0;
                    });
                    test_println!("prev={:#x}", prev)
                });
            });
            let t2 = thread::spawn(move || {
                let (causalities, stack) = &*shared2;
                stack.push(1, |prev| {
                    causalities[1].with_mut(|c| unsafe {
                        *c = 1;
                    });
                    test_println!("prev={:#x}", prev)
                });
            });

            let (causalities, stack) = &*shared;
            let mut idx = stack.pop_all();
            while idx == None {
                idx = stack.pop_all();
                thread::yield_now();
            }
            let idx = idx.unwrap();
            causalities[idx].with(|val| unsafe {
                assert_eq!(
                    *val, idx,
                    "CausalCell write must happen-before index is pushed to the stack!"
                );
            });

            t1.join().unwrap();
            t2.join().unwrap();
        });
    }
}
