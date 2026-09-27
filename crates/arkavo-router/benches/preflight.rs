//! Benchmarks for PreflightModerator - Target: < 5ms latency
//!
//! Per LLM integration testing requirements (docs/llm-integration-testing-requirements.md):
//! - Moderation check must add < 5ms to total request time on M4

#![allow(clippy::semicolon_if_nothing_returned)]

use arkavo_router::preflight::{PolicyId, PreflightFeature, PreflightModerator};
use arkavo_torg_circuits::CircuitFeature;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use torg_core::{BoolOp, Graph, Node, Source};

/// Build a NOT circuit: output = NOT(input0)
/// NOR(A, A) = NOT(A)
fn build_not_circuit() -> Graph {
    Graph {
        inputs: vec![0],
        nodes: vec![Node::new(1, BoolOp::Nor, Source::Id(0), Source::Id(0))],
        outputs: vec![1],
    }
}

fn bench_preflight_check(c: &mut Criterion) {
    let moderator = PreflightModerator::new();
    // Register 3 policies (typical production setup)
    moderator.register_graph(
        PolicyId::new("block_pii"),
        build_not_circuit(),
        vec![PreflightFeature::InputContainsPII],
    );
    moderator.register_graph(
        PolicyId::new("block_sql"),
        build_not_circuit(),
        vec![PreflightFeature::InputContainsSQLKeywords],
    );
    moderator.register_graph(
        PolicyId::new("block_shell"),
        build_not_circuit(),
        vec![PreflightFeature::InputContainsShellCommands],
    );

    let clean_input = "What is the weather today?";
    let pii_input = "My SSN is 123-45-6789";
    let long_input = "x".repeat(100_000); // Match 100k test case

    c.bench_function("preflight_check_clean_3_policies", |b| {
        b.iter(|| moderator.check(black_box(clean_input)))
    });

    c.bench_function("preflight_check_blocked_pii", |b| {
        b.iter(|| moderator.check(black_box(pii_input)))
    });

    c.bench_function("preflight_check_100k_chars", |b| {
        b.iter(|| moderator.check(black_box(&long_input)))
    });
}

fn bench_feature_extraction(c: &mut Criterion) {
    let input = "My SSN is 123-45-6789 and card is 4111-1111-1111-1111";

    c.bench_function("feature_pii_extract", |b| {
        b.iter(|| PreflightFeature::InputContainsPII.extract(black_box(input)))
    });

    c.bench_function("feature_sql_extract", |b| {
        b.iter(|| PreflightFeature::InputContainsSQLKeywords.extract(black_box(input)))
    });

    c.bench_function("feature_shell_extract", |b| {
        b.iter(|| PreflightFeature::InputContainsShellCommands.extract(black_box(input)))
    });

    c.bench_function("feature_base64_extract", |b| {
        b.iter(|| PreflightFeature::InputContainsBase64.extract(black_box(input)))
    });
}

criterion_group!(benches, bench_preflight_check, bench_feature_extraction);
criterion_main!(benches);
