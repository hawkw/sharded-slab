use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::{
    sync::{Arc, Barrier},
    time::Duration,
};

fn big_vec(c: &mut Criterion) {
    const SIZE: &'static [usize] = &[512, 1024, 4086, 10512];
    let mut group = c.benchmark_group("big_vec");

    for i in SIZE {
        group.bench_with_input(BenchmarkId::new("sharded_slab::Slab", i), i, |b, &size| {
            let slab = sharded_slab::Slab::new();
            b.iter(|| {
                for _ in 0..5 {
                    let mut vec = Vec::new();
                    for i in 0..size {
                        vec.push(i);
                    }
                    let idx = slab.insert(vec).unwrap();
                    drop(slab.take(idx));
                }
            })
        });
        group.bench_with_input(BenchmarkId::new("sharded_slab::Pool", i), i, |b, &size| {
            let pool = sharded_slab::Pool::new();
            b.iter(|| {
                for _ in 0..5 {
                    let idx = pool
                        .create(|vec: &mut Vec<usize>| {
                            for i in 0..size {
                                vec.push(i);
                            }
                        })
                        .unwrap();
                    assert!(pool.clear(idx));
                }
            })
        });
    }
    group.finish();
}

criterion_group!(benches, big_vec);
criterion_main!(benches);
