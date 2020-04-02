use std::{
    sync::{Arc, Barrier},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct MultithreadedBench<T> {
    start: Arc<Barrier>,
    end: Arc<Barrier>,
    slab: Arc<T>,
}

impl<T: Send + Sync + 'static> MultithreadedBench<T> {
    pub fn new(slab: Arc<T>) -> Self {
        Self::with_threads(slab, 5)
    }

    pub fn with_threads(slab: Arc<T>, threads: usize) -> Self {
        Self {
            start: Arc::new(Barrier::new(threads)),
            end: Arc::new(Barrier::new(threads)),
            slab,
        }
    }

    pub fn thread(&self, f: impl FnOnce(&Barrier, &T) + Send + 'static) -> &Self {
        let start = self.start.clone();
        let end = self.end.clone();
        let slab = self.slab.clone();
        thread::spawn(move || {
            f(&*start, &*slab);
            end.wait();
        });
        self
    }

    pub fn run(&self) -> Duration {
        self.start.wait();
        let t0 = Instant::now();
        self.end.wait();
        t0.elapsed()
    }
}
