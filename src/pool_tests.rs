use crate::{clear::Clear, tests::util::*, Pool};
use std::{sync::{ Arc, Mutex }, thread};

#[derive(Default)]
struct DontDropMe {
    drop: Mutex<bool>,
    clear: Mutex<bool>,
}

impl DontDropMe {
    fn new() -> Self {
        Self {
            drop: Mutex::new(false),
            clear: Mutex::new(false),
        }
    }
}

impl Drop for DontDropMe {
    fn drop(&mut self) {
        *self.drop.lock().unwrap() = true;
    }
}

impl Clear for DontDropMe {
    fn clear(&mut self) {
        *self.clear.lock().unwrap() = true;
    }
}

#[test]
fn dont_drop() {
    run_model("dont_drop", || {
        let pool = Arc::new(Pool::new());
        let item1 = Arc::new(DontDropMe::new());
        let item2 = Arc::new(DontDropMe::new());

        let p = pool.clone();
        let i = item1.clone();
        let t1 = thread::spawn(move || {
            p.create(|item: &mut Arc<DontDropMe>| *item = i.clone())
                .expect("Create");
        });

        let p = pool.clone();
        let idx = p
            .create(|item: &mut Arc<DontDropMe>| *item = item2.clone())
            .expect("Create");

        t1.join().expect("Failed to join thread 1");
        assert!(!*item1.drop.lock().unwrap());
        assert!(*item1.clear.lock().unwrap());

        p.clear(idx);
        assert!(!*item2.drop.lock().unwrap());
        assert!(*item2.clear.lock().unwrap());
    });
}
