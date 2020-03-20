use crate::{clear::Clear, tests::util::*, Pool};
use loom::{thread, sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
} };

#[derive(Default, Debug)]
struct DontDropMe {
    id: usize,
    drop: AtomicBool,
    clear: AtomicBool,
}

impl DontDropMe {
    fn new(id: usize) -> Self {
        Self {
            id,
            drop: AtomicBool::new(false),
            clear: AtomicBool::new(false),
        }
    }
}

impl Drop for DontDropMe {
    fn drop(&mut self) {
        test_println!("-> DontDropMe drop: dropping data {:?}", self.id);
        self.drop.store(true, Ordering::SeqCst);
    }
}

impl Clear for Arc<DontDropMe> {
    fn clear(&mut self) {
        test_println!("-> DontDropMe clear: clearing data {:?}", self.id);
        self.clear.store(true, Ordering::SeqCst);
    }
}

#[test]
fn dont_drop() {
    run_model("dont_drop", || {
        let pool: Arc<Pool<Arc<DontDropMe>>> = Arc::new(Pool::new());
        let item1 = Arc::new(DontDropMe::new(1));
        let item2 = Arc::new(DontDropMe::new(2));

        let p = pool.clone();
        let value = item1.clone();
        let t1 = thread::spawn(move || {
            test_println!("-> dont_drop: Inserting into pool {}", value.id);
            let idx = p.create(|item: &mut Arc<DontDropMe>| *item = value.clone())
                .expect("Create");
            let _guard = p.get(idx);
        });

        let p = pool.clone();
        let value = item2.clone();
        test_println!("-> dont_drop: Inserting into pool {}", value.id);
        let idx = p
            .create(move |item| *item = value.clone())
            .expect("Create");

        test_println!("-> dont_drop: clearing idx: {}", idx);
        p.clear(idx);

        assert!(!item2.drop.load(Ordering::SeqCst));
        assert!(item2.clear.load(Ordering::SeqCst));

        t1.join().expect("Failed to join thread 1");
        assert!(!item1.drop.load(Ordering::SeqCst));
        assert!(item1.clear.load(Ordering::SeqCst));
    });
}
