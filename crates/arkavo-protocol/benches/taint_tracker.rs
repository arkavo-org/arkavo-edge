//! Per-call overhead of taint tracking.
//!
//! The sequence-integrity invariant is <50µs added per tool call. Ingestion
//! and propagation sit on that path; recording a call touches the graph. This
//! bench measures each separately so a regression points at the step that
//! caused it.

use std::fmt::Write as _;
use std::hint::black_box;
use std::sync::Arc;

use arkavo_protocol::taint::{SourceKind, TaintSource, Transformation};
use arkavo_protocol::taint_inference::RegexInferencer;
use arkavo_protocol::taint_tracker::DataTaintTracker;
use criterion::{Criterion, criterion_group, criterion_main};
use serde_json::json;

/// A tool result of the size an agent actually handles, carrying one secret.
fn payload() -> String {
    let mut text = String::with_capacity(4096);
    for i in 0..80 {
        // Writing to a String is infallible; the Result exists only because
        // `write!` is generic over sinks that can fail.
        let _ = writeln!(
            text,
            "line {i}: the quick brown fox jumps over the lazy dog"
        );
    }
    // Built rather than written down, so no secret-shaped literal is committed.
    let prefix: String = ['s', 'k'].iter().collect();
    let body: String = (0..24)
        .map(|i| char::from(b'a' + ((i * 7 + 3) % 26) as u8))
        .collect();
    let _ = writeln!(text, "api key {prefix}-{body}");
    text
}

fn bench_taint(c: &mut Criterion) {
    let tracker = DataTaintTracker::new("bench-session");
    let source = TaintSource::new(SourceKind::ToolResult, "read_file");
    let text = payload();

    c.bench_function("ingest_4kb", |b| {
        b.iter(|| black_box(tracker.ingest(black_box(&source), black_box(&text))));
    });

    let ingested = tracker.ingest(&source, &text);
    c.bench_function("transform", |b| {
        b.iter(|| {
            black_box(tracker.transform(black_box(&[&ingested]), Transformation::Encode, "base64"));
        });
    });

    c.bench_function("after_inference", |b| {
        b.iter(|| black_box(tracker.after_inference(black_box(&[&ingested]), "gemma-e2b")));
    });

    // A fresh tracker per iteration keeps the graph from growing across the
    // run. The detector is shared so the timed region drops an `Arc` clone
    // rather than a compiled pattern set, whose teardown would otherwise
    // dominate the measurement.
    let params = json!({"path": "/etc/hosts", "limit": 100});
    let shared: Arc<RegexInferencer> = Arc::new(RegexInferencer::new());
    c.bench_function("record_call", |b| {
        b.iter_batched(
            || DataTaintTracker::new("bench-session").with_inferencer(shared.clone()),
            |t| {
                black_box(
                    t.record_call(black_box("read_file"), black_box(&params), &[], &ingested)
                        .expect("root call"),
                )
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

criterion_group!(benches, bench_taint);
criterion_main!(benches);
