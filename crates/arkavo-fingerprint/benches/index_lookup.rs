//! KP-011: the reference tier's cost on the per-call path.
//!
//! The number that matters is the composite: this tier runs inside the same
//! `ingest` as the pattern detector, and the 50µs budget is shared. Measuring
//! the hash-map probe alone would flatter it — the real cost is one keyed hash
//! per shingle, which scales with span size the way the regex pass does.

use std::fmt::Write as _;
use std::hint::black_box;
use std::sync::Arc;

use arkavo_fingerprint::{IndexKey, ReferenceIndex, ReferenceTier, match_span};
use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
use criterion::{Criterion, criterion_group, criterion_main};

/// Mirrors the 4KB span the taint bench uses, so the two are comparable.
fn span_4kb() -> String {
    let mut text = String::with_capacity(4400);
    for i in 0..80 {
        let _ = writeln!(
            text,
            "line {i}: the quick brown fox jumps over the lazy dog"
        );
    }
    text
}

fn corpus_index(key: &IndexKey) -> ReferenceIndex {
    let mut builder = ReferenceIndex::builder(key, "1.0.0");
    // A corpus large enough that the probe is a real hash-map lookup rather
    // than a cache-resident handful of entries.
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

fn benches(c: &mut Criterion) {
    let key = Arc::new(IndexKey::derive(&[7u8; 32], "bench-corpus").expect("derive"));
    let index = Arc::new(corpus_index(&key));
    let tier = ReferenceTier::loaded(index.clone(), key.clone());
    let text = span_4kb();
    let hit = "document 42 concerning the acquisition of holdings number 42 closing in the quarter";

    c.bench_function("keyed_hash_one_shingle", |b| {
        b.iter(|| black_box(key.hash(black_box("the quick brown fox jumps"))));
    });

    c.bench_function("tier_examine_hit", |b| {
        b.iter(|| black_box(tier.examine(black_box(hit))));
    });

    c.bench_function("tier_examine_miss", |b| {
        b.iter(|| {
            black_box(tier.examine(black_box(
                "entirely unrelated prose about the weather this week",
            )))
        });
    });

    c.bench_function("match_span_4kb", |b| {
        b.iter(|| black_box(match_span(&index, &key, black_box(&text))));
    });
}

criterion_group!(index_lookup, benches);
criterion_main!(index_lookup);
