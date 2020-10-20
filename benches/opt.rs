use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

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
    group.finish();
}

criterion_group!(clear_remove, clear);
criterion_main!(clear_remove);
