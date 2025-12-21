//! Benchmark to verify Titan Monitor overhead stays under 5µs
//!
//! Run with: `cargo bench -p arkavo-titan`

use std::collections::HashMap;
use std::sync::Arc;

use arkavo_sbe::InvariantLayer;
use arkavo_titan::{Graph, TitanConfig, TitanMonitor};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use torg_core::{Builder, Token};

/// Build a representative policy graph (4 inputs, 2 outputs, 5 nodes)
fn build_benchmark_graph() -> Graph {
    let mut builder = Builder::new();

    // 4 inputs
    for i in 0..4 {
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(i)).unwrap();
    }

    // Node 4: OR(0, 1)
    builder.push(Token::NodeStart).unwrap();
    builder.push(Token::Id(4)).unwrap();
    builder.push(Token::Or).unwrap();
    builder.push(Token::Id(0)).unwrap();
    builder.push(Token::Id(1)).unwrap();
    builder.push(Token::NodeEnd).unwrap();

    // Node 5: NOR(2, 3) = NOT(2 OR 3)
    builder.push(Token::NodeStart).unwrap();
    builder.push(Token::Id(5)).unwrap();
    builder.push(Token::Nor).unwrap();
    builder.push(Token::Id(2)).unwrap();
    builder.push(Token::Id(3)).unwrap();
    builder.push(Token::NodeEnd).unwrap();

    // Node 6: XOR(4, 5)
    builder.push(Token::NodeStart).unwrap();
    builder.push(Token::Id(6)).unwrap();
    builder.push(Token::Xor).unwrap();
    builder.push(Token::Id(4)).unwrap();
    builder.push(Token::Id(5)).unwrap();
    builder.push(Token::NodeEnd).unwrap();

    // Node 7: OR(4, 6)
    builder.push(Token::NodeStart).unwrap();
    builder.push(Token::Id(7)).unwrap();
    builder.push(Token::Or).unwrap();
    builder.push(Token::Id(4)).unwrap();
    builder.push(Token::Id(6)).unwrap();
    builder.push(Token::NodeEnd).unwrap();

    // 2 outputs
    builder.push(Token::OutputDecl).unwrap();
    builder.push(Token::Id(6)).unwrap();
    builder.push(Token::OutputDecl).unwrap();
    builder.push(Token::Id(7)).unwrap();

    builder.finish().unwrap()
}

fn bench_raw_evaluate(c: &mut Criterion) {
    let graph = build_benchmark_graph();
    let inputs: HashMap<u16, bool> = HashMap::from([
        (0, true),
        (1, false),
        (2, true),
        (3, false),
    ]);

    c.bench_function("raw_evaluate", |b| {
        b.iter(|| torg_core::evaluate(black_box(&graph), black_box(&inputs)))
    });
}

fn bench_monitor_evaluate(c: &mut Criterion) {
    let graph = build_benchmark_graph();
    let inputs: HashMap<u16, bool> = HashMap::from([
        (0, true),
        (1, false),
        (2, true),
        (3, false),
    ]);

    let invariants = Arc::new(InvariantLayer::new());
    let config = TitanConfig {
        enable_drift_detection: true,
        drift_min_samples: 1000, // Avoid drift detection triggering during bench
        ..Default::default()
    };
    let (mut monitor, _rx) = TitanMonitor::new(invariants, config);

    c.bench_function("monitor_evaluate", |b| {
        b.iter(|| monitor.evaluate(black_box(&graph), black_box(&inputs)))
    });
}

fn bench_monitor_overhead(c: &mut Criterion) {
    let graph = build_benchmark_graph();
    let inputs: HashMap<u16, bool> = HashMap::from([
        (0, true),
        (1, false),
        (2, true),
        (3, false),
    ]);

    let invariants = Arc::new(InvariantLayer::new());
    let config = TitanConfig {
        enable_drift_detection: true,
        drift_min_samples: 1000,
        ..Default::default()
    };
    let (mut monitor, _rx) = TitanMonitor::new(invariants, config);

    let mut group = c.benchmark_group("overhead_comparison");

    group.bench_function("raw", |b| {
        b.iter(|| torg_core::evaluate(black_box(&graph), black_box(&inputs)))
    });

    group.bench_function("monitored", |b| {
        b.iter(|| monitor.evaluate(black_box(&graph), black_box(&inputs)))
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_raw_evaluate,
    bench_monitor_evaluate,
    bench_monitor_overhead
);
criterion_main!(benches);
