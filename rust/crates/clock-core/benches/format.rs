use clock_core::format::{CompiledFormat, PRESETS};
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
fn benchmark(c: &mut Criterion) {
    let sample = chrono::DateTime::parse_from_rfc3339("2027-01-01T12:34:56+01:00").unwrap();
    let input = PRESETS[2].format;
    let compiled = CompiledFormat::parse(input).unwrap();
    c.bench_function("format/parse", |b| {
        b.iter(|| CompiledFormat::parse(black_box(input)).unwrap())
    });
    c.bench_function("format/render", |b| {
        b.iter(|| compiled.render(black_box(sample)).unwrap())
    });
}
criterion_group! { name = benches; config = Criterion::default().sample_size(20).warm_up_time(std::time::Duration::from_millis(500)).measurement_time(std::time::Duration::from_secs(1)); targets = benchmark }
criterion_main!(benches);
