use crate::{clear::Clear, tests::util::*, Pool};
use std::{
    sync::{Arc, Mutex},
    thread,
};

struct TinyConfig;

impl crate::Config for TinyConfig {
    const INITIAL_PAGE_SIZE: usize = 4;
}

#[derive(Default)]
struct DontDropMe {
    drop: Mutex<bool>,
    clear: Mutex<bool>,
}

impl DontDropMe {
    fn new() -> Self {
        Self {
            drop: Mutex::new(true),
            clear: Mutex::new(true),
        }
    }

    fn print_state(&mut self) {
        println!("{:?}, {:?}", self.drop, self.clear);
    }
}

impl Drop for DontDropMe {
    fn drop(&mut self) {
        let val = self.drop.lock().unwrap();
        *val = true;
    }
}

impl Clear for DontDropMe {
    fn clear(&mut self) {
        let val = self.clear.lock().unwrap();
        *val = true;
    }
}

#[test]
fn dont_drop() {
    run_model("dont_drop", || {
        let pool = Arc::new(Pool::new());
        let item1 = DontDropMe::new();
        let item2 = DontDropMe::new();

        let p = pool.clone();
        let t1 = thread::spawn(|| {
            p.create(|item: &mut DontDropMe| *item = item1)
                .expect("Create");
        });

        let p = pool.clone();
        let idx = p
            .create(|item: &mut DontDropMe| *item = item2)
            .expect("Create");

        t1.join();
        assert!(!*item1.drop.lock().unwrap());
        assert!(*item1.clear.lock().unwrap());

        p.clear(idx);
        assert!(!*item2.drop.lock().unwrap());
        assert!(*item2.clear.lock().unwrap());
    });
}
