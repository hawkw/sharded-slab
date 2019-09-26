use crate::Slab;
use loom::sync::Arc;
use loom::thread;

mod idx {
    use crate::{
        page::{self, slot},
        Pack, Tid,
    };
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn tid_roundtrips(tid in 0usize..Tid::BITS) {
            let tid = Tid::from_usize(tid);
            let packed = tid.pack(0);
            assert_eq!(tid, Tid::from_packed(packed));
        }

        #[test]
        fn idx_roundtrips(
            tid in 0usize..Tid::BITS,
            gen in 0usize..slot::Generation::BITS,
            pidx in 0usize..page::Index::BITS,
            poff in 0usize..page::Offset::BITS,
        ) {
            let tid = Tid::from_usize(tid);
            let gen = slot::Generation::from_usize(gen);
            let pidx = page::Index::from_usize(pidx);
            let poff = page::Offset::from_usize(poff);
            let packed = tid.pack(gen.pack(pidx.pack(poff.pack(0))));
            assert_eq!(poff, page::Offset::from_packed(packed));
            assert_eq!(pidx, page::Index::from_packed(packed));
            assert_eq!(gen, slot::Generation::from_packed(packed));
            assert_eq!(tid, Tid::from_packed(packed));
        }
    }
}

#[test]
fn local_remove() {
    loom::model(|| {
        let slab = Arc::new(Slab::new());

        let s = slab.clone();
        let t1 = thread::spawn(move || {
            let idx = s.insert(1).expect("insert");
            assert_eq!(s.get(idx), Some(&1));
            assert_eq!(s.remove(idx), Some(1));
            assert_eq!(s.get(idx), None);
            let idx = s.insert(2).expect("insert");
            assert_eq!(s.get(idx), Some(&2));
            assert_eq!(s.remove(idx), Some(2));
            assert_eq!(s.get(idx), None);
        });

        let s = slab.clone();
        let t2 = thread::spawn(move || {
            let idx = s.insert(3).expect("insert");
            assert_eq!(s.get(idx), Some(&3));
            assert_eq!(s.remove(idx), Some(3));
            assert_eq!(s.get(idx), None);
            let idx = s.insert(4).expect("insert");
            assert_eq!(s.get(idx), Some(&4));
            assert_eq!(s.remove(idx), Some(4));
            assert_eq!(s.get(idx), None);
        });

        let s = slab;
        let idx1 = s.insert(5).expect("insert");
        assert_eq!(s.get(idx1), Some(&5));
        let idx2 = s.insert(6).expect("insert");
        assert_eq!(s.get(idx2), Some(&6));
        assert_eq!(s.remove(idx1), Some(5));
        assert_eq!(s.get(idx1), None);
        assert_eq!(s.get(idx2), Some(&6));
        assert_eq!(s.remove(idx2), Some(6));
        assert_eq!(s.get(idx2), None);

        t1.join().expect("thread 1 should not panic");
        t2.join().expect("thread 2 should not panic");
    });
}

#[test]
fn remove_remote() {
    loom::model(|| {
        let slab = Arc::new(Slab::new());

        let idx1 = slab.insert(1).expect("insert");
        assert_eq!(slab.get(idx1), Some(&1));

        let idx2 = slab.insert(2).expect("insert");
        assert_eq!(slab.get(idx2), Some(&2));

        let idx3 = slab.insert(3).expect("insert");
        assert_eq!(slab.get(idx3), Some(&3));

        let s = slab.clone();
        let t1 = thread::spawn(move || {
            assert_eq!(s.get(idx2), Some(&2));
            assert_eq!(s.remove(idx2), Some(2));
        });

        let s = slab.clone();
        let t2 = thread::spawn(move || {
            assert_eq!(s.get(idx3), Some(&3));
            assert_eq!(s.remove(idx3), Some(3));
        });

        t1.join().expect("thread 1 should not panic");
        t2.join().expect("thread 2 should not panic");

        assert_eq!(slab.get(idx1), Some(&1));
        assert_eq!(slab.get(idx2), None);
        assert_eq!(slab.get(idx3), None);
    });
}

#[test]
fn remove_remote_and_reuse() {
    loom::model(|| {
        let slab = Arc::new(Slab::builder().max_pages(1).initial_page_size(4).finish());

        let idx1 = slab.insert(1).expect("insert");
        let idx2 = slab.insert(2).expect("insert");
        let idx3 = slab.insert(3).expect("insert");
        let idx4 = slab.insert(4).expect("insert");

        assert_eq!(slab.get(idx1), Some(&1));
        assert_eq!(slab.get(idx2), Some(&2));
        assert_eq!(slab.get(idx3), Some(&3));
        assert_eq!(slab.get(idx4), Some(&4));

        let s = slab.clone();
        let t1 = thread::spawn(move || {
            println!("tid is: {:?}", crate::Tid::current());
            assert_eq!(s.remove(idx1), Some(1));
        });

        let s = slab.clone();
        let t2 = thread::spawn(move || {
            println!("tid is: {:?}", crate::Tid::current());
            assert_eq!(s.remove(idx2), Some(2));
        });

        t1.join().expect("thread 1 should not panic");
        t2.join().expect("thread 2 should not panic");

        let idx1 = slab.insert(5).expect("insert");
        let idx2 = slab.insert(6).expect("insert");

        assert_eq!(slab.get(idx1), Some(&5));
        assert_eq!(slab.get(idx2), Some(&6));
        assert_eq!(slab.get(idx3), Some(&3));
        assert_eq!(slab.get(idx4), Some(&4));
    });
}
