use crate::{clear::Clear, tests::util::*, Pool};
use loom::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
};

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
        let pool: Pool<Arc<DontDropMe>> = Pool::new();
        let item1 = Arc::new(DontDropMe::new(1));
        test_println!("-> dont_drop: Inserting into pool {}", item1.id);
        let value = item1.clone();
        let idx = pool
            .create(move |item| *item = value.clone())
            .expect("Create");

        test_println!("-> dont_drop: clearing idx: {}", idx);
        pool.clear(idx);

        assert!(!item1.drop.load(Ordering::SeqCst));
        assert!(item1.clear.load(Ordering::SeqCst));
    });
}

#[test]
fn dont_drop_across_threads() {
    run_model("dont_drop_across_threads", || {
        let pool: Arc<Pool<Arc<DontDropMe>>> = Arc::new(Pool::new());

        let item1 = Arc::new(DontDropMe::new(1));
        let value = item1.clone();
        let idx1 = pool
            .create(move |item| *item = value.clone())
            .expect("Create");

        let p = pool.clone();
        let item = item1.clone();
        let t1 = thread::spawn(move || {
            assert_eq!(p.get(idx1).unwrap().id, item.id);
        });

        let item = item1.clone();
        assert!(item.drop.load(Ordering::SeqCst));
        assert!(item.clear.load(Ordering::SeqCst));

        t1.join().expect("thread 1 unable to join");
        pool.clear(idx1);

        assert!(!item1.drop.load(Ordering::SeqCst));
        assert!(item1.clear.load(Ordering::SeqCst));
    })
}
