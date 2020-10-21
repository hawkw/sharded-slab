use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use std::time::{Duration, Instant};

fn clear(c: &mut Criterion) {
    let mut group = c.benchmark_group("clear");
    group.bench_function("clear_with_key", |b| {
        let pool = sharded_slab::Pool::<String>::new();
        b.iter_batched(
            || {
                let mut guard = pool.create().unwrap();
                guard.push_str("hello world");
                guard
            },
            |guard| {
                pool.clear(guard.key());
                drop(guard);
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("mark_clear", |b| {
        let pool = sharded_slab::Pool::<String>::new();
        b.iter_batched(
            || {
                let mut guard = pool.create().unwrap();
                guard.push_str("hello world");
                guard
            },
            |guard| {
                guard.clear_on_drop();
                drop(guard);
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("string", |b| {
        b.iter_batched(
            || String::from("hello world"),
            |guard| {
                drop(guard);
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("arc_string", |b| {
        b.iter_batched(
            || std::sync::Arc::new(String::from("hello world")),
            |guard| {
                drop(guard);
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn push_str(c: &mut Criterion) {
    let mut group = c.benchmark_group("push_str");

    let pool = sharded_slab::Pool::<String>::new();

    group.bench_function("pool", |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::from_secs(0);
            for _ in 0..iters {
                let now = Instant::now();
                let mut string = pool.create().unwrap();
                string.push_str("hello world");
                elapsed += now.elapsed();
                string.clear_on_drop();
                drop(string);
            }
            elapsed
        });
    });
    group.bench_function("string", |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::from_secs(0);
            for _ in 0..iters {
                let now = Instant::now();
                let mut string = String::new();
                string.push_str("hello world");
                elapsed += now.elapsed();
                drop(string);
            }
            elapsed
        });
    });
    group.finish();
}
criterion_group!(benches, clear, push_str);
criterion_main!(benches);
