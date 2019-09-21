use loom::sync::{Arc, Mutex};
use loom::thread;
use mislab::Slab;

#[test]
fn local_remove() {
    loom::model(|| {
        let slab = Arc::new(Slab::builder().with_max_threads(4).finish());

        let s = slab.clone();
        let t1 = thread::spawn(move || {
            let idx = s.insert(1).expect("insert");
            assert_eq!(s.get(idx), Some(&1));
            s.remove(idx);
            assert_eq!(s.get(idx), None);
            let idx = s.insert(2).expect("insert");
            assert_eq!(s.get(idx), Some(&2));
            s.remove(idx);
            assert_eq!(s.get(idx), None);
        });

        let s = slab.clone();
        let t2 = thread::spawn(move || {
            let idx = s.insert(3).expect("insert");
            assert_eq!(s.get(idx), Some(&3));
            s.remove(idx);
            assert_eq!(s.get(idx), None);
            let idx = s.insert(4).expect("insert");
            assert_eq!(s.get(idx), Some(&4));
            s.remove(idx);
            assert_eq!(s.get(idx), None);
        });

        let s = slab;
        let idx1 = s.insert(5).expect("insert");
        assert_eq!(s.get(idx1), Some(&5));
        let idx2 = s.insert(6).expect("insert");
        assert_eq!(s.get(idx2), Some(&6));
        s.remove(idx1);
        assert_eq!(s.get(idx1), None);
        assert_eq!(s.get(idx2), Some(&6));
        s.remove(idx2);
        assert_eq!(s.get(idx2), None);

        t1.join().expect("thread 1 should not panic");
        t2.join().expect("thread 2 should not panic");
    });
}

#[test]
fn remove_remote() {
    loom::model(|| {
        let slab = Arc::new(Slab::builder().with_max_threads(4).finish());

        let idx1 = slab.insert(1).expect("insert");
        assert_eq!(slab.get(idx1), Some(&1));

        let idx2 = slab.insert(2).expect("insert");
        assert_eq!(slab.get(idx2), Some(&2));

        let idx3 = slab.insert(3).expect("insert");
        assert_eq!(slab.get(idx3), Some(&3));

        let s = slab.clone();
        let t1 = thread::spawn(move || {
            assert_eq!(s.get(idx2), Some(&2));
            s.remove(idx2)
        });

        let s = slab.clone();
        let t2 = thread::spawn(move || {
            assert_eq!(s.get(idx3), Some(&3));
            s.remove(idx3)
        });

        t1.join().expect("thread 1 should not panic");
        t2.join().expect("thread 2 should not panic");

        assert_eq!(slab.get(idx1), Some(&1));
        assert_eq!(slab.get(idx2), None);
        assert_eq!(slab.get(idx3), None);
    });
}
