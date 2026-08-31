//! SENT-007, SENT-016: what holdback costs, published separately.
//!
//! Two numbers, deliberately not one. The per-call synchronous overhead is what
//! the 50µs invariant caps; the holdback latency is what a consumer waits for a
//! window to clear. Reporting them together would let a fast cascade hide a slow
//! release, or a slow cascade look acceptable because the window was small.

use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use arkavo_fingerprint::{IndexKey, NearDuplicateTier, ReferenceIndex, ReferenceTier};
use arkavo_protocol::RegexInferencer;
use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
use arkavo_sentinel::{Cascade, CascadeTier, Holdback, PatternTier};
use criterion::{Criterion, criterion_group, criterion_main};

fn corpus_index(key: &IndexKey) -> ReferenceIndex {
    let mut builder = ReferenceIndex::builder(key, "1.0.0");
    for doc in 0..500 {
        builder.add_document(
            key,
            &format!(
                "document {doc} concerning the acquisition of holdings number {doc} \
                 closing in the quarter pending board approval and counsel review"
            ),
            DataCategory::Internal,
            SensitivityLevel::Confidential,
            "board-minutes",
        );
    }
    builder.build()
}

/// A completion the holdback path streams, in chunks a model would produce.
fn completion() -> Vec<String> {
    (0..64)
        .map(|i| format!("token {i} of a completion that is being streamed to a consumer. "))
        .collect()
}

fn benches(c: &mut Criterion) {
    let key = Arc::new(IndexKey::derive(&[7u8; 32], "bench-corpus").expect("derive"));
    let index = Arc::new(corpus_index(&key));
    let cascade = Arc::new(
        Cascade::new("1.0.0")
            .with_tier(Arc::new(PatternTier::new(Arc::new(RegexInferencer::new()))))
            .with_tier(Arc::new(ReferenceTier::loaded(index, key)) as Arc<dyn CascadeTier>)
            .with_tier(Arc::new(NearDuplicateTier::unloaded(
                "no near index in this bench",
            ))),
    );
    let span =
        "document 42 concerning the acquisition of holdings number 42 closing in the quarter";

    // The number the 50µs invariant caps.
    c.bench_function("cascade_per_call_overhead", |b| {
        b.iter(|| black_box(cascade.inspect(black_box(span))));
    });

    // What a consumer waits for one window to clear: buffer plus inspection.
    c.bench_function("holdback_window_latency", |b| {
        b.iter(|| {
            let mut holdback = Holdback::default();
            let mut released = 0usize;
            for chunk in completion() {
                holdback.push(&chunk);
                while let Some(window) = holdback.take_window() {
                    let evidence = cascade.inspect_until(
                        &window.inspect,
                        Instant::now() + arkavo_sentinel::CASCADE_BUDGET,
                    );
                    if evidence.findings().next().is_some() {
                        holdback.block();
                        break;
                    }
                    released += holdback.release().len();
                }
            }
            holdback.finish();
            while let Some(window) = holdback.take_window() {
                black_box(cascade.inspect(&window.inspect));
                released += holdback.release().len();
            }
            black_box(released)
        });
    });
}

criterion_group!(holdback, benches);
criterion_main!(holdback);
