use criterion::{criterion_group, criterion_main, Criterion};
use laptop_selector::prepare_laptop_requests_router;

pub fn initialization_benchmark(c: &mut Criterion) {
    c.bench_function("generate routes and data", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| prepare_laptop_requests_router())
    });
}

criterion_group!(benches, initialization_benchmark);
criterion_main!(benches);
