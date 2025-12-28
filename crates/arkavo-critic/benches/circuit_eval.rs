//! Benchmarks for CircuitCheck evaluation performance.
//!
//! Target: sub-microsecond total latency (feature extraction + evaluation).

use arkavo_critic::{CircuitCheck, FeatureId, PolicyId, VerificationInput};
use arkavo_llm::ProviderResponse;
use arkavo_llm::tool_parser::ParsedToolCall;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use torg_core::{BoolOp, Graph, Node, Source, evaluate_into};
use torg_serde::to_bytes;

/// Build a simple OR circuit: output = input0 OR input1
fn build_or_circuit() -> Graph {
    Graph {
        inputs: vec![0, 1],
        nodes: vec![Node::new(2, BoolOp::Or, Source::Id(0), Source::Id(1))],
        outputs: vec![2],
    }
}

/// Build a policy circuit: NOT(guest AND write)
/// This blocks guest users from writing.
fn build_block_guest_writes_circuit() -> Graph {
    Graph {
        inputs: vec![0, 1],
        nodes: vec![
            // Node 2: NOR(0, 0) = NOT(guest)
            Node::new(2, BoolOp::Nor, Source::Id(0), Source::Id(0)),
            // Node 3: NOR(1, 1) = NOT(write)
            Node::new(3, BoolOp::Nor, Source::Id(1), Source::Id(1)),
            // Node 4: NOR(NOT_guest, NOT_write) = guest AND write
            Node::new(4, BoolOp::Nor, Source::Id(2), Source::Id(3)),
            // Node 5: NOR(4, 4) = NOT(guest AND write) = PASS unless both true
            Node::new(5, BoolOp::Nor, Source::Id(4), Source::Id(4)),
        ],
        outputs: vec![5],
    }
}

/// Build a complex circuit with 10 chained nodes
fn build_chain_circuit(depth: usize) -> Graph {
    let mut nodes = Vec::with_capacity(depth);

    // First node takes input 0
    nodes.push(Node::new(1, BoolOp::Or, Source::Id(0), Source::True));

    // Chain subsequent nodes
    for i in 2..=depth {
        nodes.push(Node::new(
            i as u16,
            BoolOp::Or,
            Source::Id((i - 1) as u16),
            Source::True,
        ));
    }

    Graph {
        inputs: vec![0],
        nodes,
        outputs: vec![depth as u16],
    }
}

fn make_response(content: &str, tool_calls: Vec<ParsedToolCall>) -> ProviderResponse {
    ProviderResponse {
        content: content.to_string(),
        reasoning_content: None,
        tool_calls,
        finish_reason: None,
    }
}

fn bench_raw_circuit_eval(c: &mut Criterion) {
    let graph = build_or_circuit();
    let inputs = [true, false];
    let mut outputs = [false];

    c.bench_function("raw_circuit_eval_simple_or", |b| {
        b.iter(|| {
            evaluate_into(black_box(&graph), black_box(&inputs), &mut outputs);
            black_box(outputs[0])
        })
    });
}

fn bench_policy_circuit_eval(c: &mut Criterion) {
    let graph = build_block_guest_writes_circuit();
    let inputs = [true, true]; // guest=true, write=true
    let mut outputs = [false];

    c.bench_function("raw_circuit_eval_policy_4_nodes", |b| {
        b.iter(|| {
            evaluate_into(black_box(&graph), black_box(&inputs), &mut outputs);
            black_box(outputs[0])
        })
    });
}

fn bench_chain_circuit_eval(c: &mut Criterion) {
    let graph_10 = build_chain_circuit(10);
    let graph_50 = build_chain_circuit(50);
    let inputs = [true];
    let mut outputs = [false];

    c.bench_function("raw_circuit_eval_chain_10", |b| {
        b.iter(|| {
            evaluate_into(black_box(&graph_10), black_box(&inputs), &mut outputs);
            black_box(outputs[0])
        })
    });

    c.bench_function("raw_circuit_eval_chain_50", |b| {
        b.iter(|| {
            evaluate_into(black_box(&graph_50), black_box(&inputs), &mut outputs);
            black_box(outputs[0])
        })
    });
}

fn bench_feature_extraction(c: &mut Criterion) {
    let input = VerificationInput::new(
        "test".into(),
        make_response(
            "```rust\nfn main() {}\n```",
            vec![ParsedToolCall {
                tool_name: "file_write".into(),
                arguments: serde_json::json!({}),
                call_id: None,
            }],
        ),
        vec![],
    );

    c.bench_function("feature_extract_has_tool_calls", |b| {
        b.iter(|| FeatureId::HasToolCalls.extract(black_box(&input)))
    });

    c.bench_function("feature_extract_response_has_code", |b| {
        b.iter(|| FeatureId::ResponseHasCode.extract(black_box(&input)))
    });

    c.bench_function("feature_extract_intent_is_write", |b| {
        b.iter(|| FeatureId::IntentIsWrite.extract(black_box(&input)))
    });
}

fn bench_circuit_check_full(c: &mut Criterion) {
    let check = CircuitCheck::new();
    let graph = build_or_circuit();
    let bytes = to_bytes(&graph);

    check
        .register(
            PolicyId::new("test_policy"),
            &bytes,
            vec![FeatureId::HasToolCalls, FeatureId::ResponseHasCode],
        )
        .unwrap();

    let input = VerificationInput::new("test".into(), make_response("```code```", vec![]), vec![]);

    // Use tokio runtime for async benchmark
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    c.bench_function("circuit_check_full_pipeline", |b| {
        b.iter(|| {
            rt.block_on(async {
                use arkavo_critic::VerificationCheck;
                check.verify(black_box(&input)).await
            })
        })
    });
}

fn bench_circuit_check_multiple_policies(c: &mut Criterion) {
    let check = CircuitCheck::new();

    // Register 5 policies
    for i in 0..5 {
        let graph = build_or_circuit();
        let bytes = to_bytes(&graph);
        check
            .register(
                PolicyId::new(format!("policy_{i}")),
                &bytes,
                vec![FeatureId::HasToolCalls, FeatureId::ResponseHasCode],
            )
            .unwrap();
    }

    let input = VerificationInput::new("test".into(), make_response("```code```", vec![]), vec![]);

    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    c.bench_function("circuit_check_5_policies", |b| {
        b.iter(|| {
            rt.block_on(async {
                use arkavo_critic::VerificationCheck;
                check.verify(black_box(&input)).await
            })
        })
    });
}

criterion_group!(
    benches,
    bench_raw_circuit_eval,
    bench_policy_circuit_eval,
    bench_chain_circuit_eval,
    bench_feature_extraction,
    bench_circuit_check_full,
    bench_circuit_check_multiple_policies,
);

criterion_main!(benches);
