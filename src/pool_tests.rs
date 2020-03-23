use crate::{clear::Clear, tests::util::*, Pool};
use loom::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
};

#[derive(Default, Debug)]
struct State {
    is_dropped: AtomicBool,
    is_cleared: AtomicBool,
    id: usize,
}

impl PartialEq for State {
    fn eq(&self, other: &State) -> bool {
        self.id.eq(&other.id)
    }
}

#[derive(Clone, Default, Debug)]
struct DontDropMe(Arc<State>);

impl PartialEq for DontDropMe {
    fn eq(&self, other: &DontDropMe) -> bool {
        self.0.eq(&other.0)
    }
}

impl DontDropMe {
    fn new(id: usize) -> (Arc<State>, Self) {
        let state = Arc::new(State {
            is_dropped: AtomicBool::new(false),
            is_cleared: AtomicBool::new(false),
            id,
        });
        (state.clone(), Self(state))
    }
}

impl Drop for DontDropMe {
    fn drop(&mut self) {
        test_println!("-> DontDropMe drop: dropping data {:?}", self.0.id);
        self.0.is_dropped.store(true, Ordering::SeqCst)
    }
}

impl Clear for DontDropMe {
    fn clear(&mut self) {
        test_println!("-> DontDropMe clear: clearing data {:?}", self.0.id);
        self.0.is_cleared.store(true, Ordering::SeqCst);
    }
}

#[test]
fn dont_drop() {
    run_model("dont_drop", || {
        let pool: Pool<DontDropMe> = Pool::new();
        let (item1, value) = DontDropMe::new(1);
        test_println!("-> dont_drop: Inserting into pool {}", item1.id);
        let idx = pool
            .create(move |item| *item = value.clone())
            .expect("Create");

        test_println!("-> dont_drop: clearing idx: {}", idx);
        pool.clear(idx);

        assert!(!item1.is_dropped.load(Ordering::SeqCst));
        assert!(item1.is_cleared.load(Ordering::SeqCst));
    });
}

#[test]
fn dont_drop_across_threads() {
    run_model("dont_drop_across_threads", || {
        let pool: Arc<Pool<DontDropMe>> = Arc::new(Pool::new());

        let (item1, value) = DontDropMe::new(1);
        let idx1 = pool
            .create(move |item| *item = value.clone())
            .expect("Create");

        let p = pool.clone();
        let test_value = item1.clone();
        let t1 = thread::spawn(move || {
            assert_eq!(p.get(idx1).unwrap().0.id, test_value.id);
        });

        assert!(!item1.is_dropped.load(Ordering::SeqCst));
        assert!(item1.is_cleared.load(Ordering::SeqCst));

        t1.join().expect("thread 1 unable to join");
        pool.clear(idx1);

        assert!(!item1.is_dropped.load(Ordering::SeqCst));
        assert!(item1.is_cleared.load(Ordering::SeqCst));
    })
}
