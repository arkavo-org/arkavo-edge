//! Dispatch-gate latency baseline (Epic 0, item 2).
//!
//! Measures the existing per-stage latencies on the policy+sequence path so a
//! 25ms dispatch-gate budget can be judged:
//! - preflight moderation (arkavo-router/src/preflight)
//! - budget check (arkavo-budget BudgetTracker::can_afford)
//! - critic pipeline checks (arkavo-critic CircuitCheck/SchemaCheck/PolicyCheck)
//!
//! Each benchmark prints a `GATE_STAGE` line with manual p50/p95 (microseconds)
//! collected from 200 samples; criterion provides the statistical mean.
//! Numbers feed docs/gate-latency-baseline.md.

#![allow(clippy::disallowed_methods, clippy::semicolon_if_nothing_returned)]

use arkavo_budget::{BudgetConfig, BudgetTracker, TokenCost};
use arkavo_critic::{
    CircuitCheck, FeatureId, PolicyCheck, SchemaCheck, VerificationCheck, VerificationInput,
};
use arkavo_llm::ProviderResponse;
use arkavo_llm::tool_parser::ParsedToolCall;
use arkavo_mcp_tools::ToolInfo;
use arkavo_router::preflight::{
    PolicyId as PreflightPolicyId, PreflightFeature, PreflightModerator,
};
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use ed25519_dalek::{Signer, SigningKey, Verifier};
use std::time::Instant;
use torg_core::{BoolOp, Graph, Node, Source};
use torg_serde::to_bytes;

const SAMPLES: usize = 200;

/// Build a NOT circuit: output = NOT(input0)
fn build_not_circuit() -> Graph {
    Graph {
        inputs: vec![0],
        nodes: vec![Node::new(1, BoolOp::Nor, Source::Id(0), Source::Id(0))],
        outputs: vec![1],
    }
}

fn build_moderator() -> PreflightModerator {
    let moderator = PreflightModerator::new();
    moderator.register_graph(
        PreflightPolicyId::new("block_pii"),
        build_not_circuit(),
        vec![PreflightFeature::InputContainsPII],
    );
    moderator.register_graph(
        PreflightPolicyId::new("block_sql"),
        build_not_circuit(),
        vec![PreflightFeature::InputContainsSQLKeywords],
    );
    moderator.register_graph(
        PreflightPolicyId::new("block_shell"),
        build_not_circuit(),
        vec![PreflightFeature::InputContainsShellCommands],
    );
    moderator
}

fn make_response(content: &str, tool_calls: Vec<ParsedToolCall>) -> ProviderResponse {
    ProviderResponse {
        content: content.to_string(),
        reasoning_content: None,
        tool_calls,
        finish_reason: None,
        inference_timing: None,
        quality_gate_retries: 0,
    }
}

fn make_tool_call() -> ParsedToolCall {
    ParsedToolCall {
        tool_name: "file_write".into(),
        arguments: serde_json::json!({"path": "/tmp/out.txt", "content": "hello"}),
        call_id: Some("call-1".into()),
    }
}

fn make_tool_info() -> ToolInfo {
    ToolInfo {
        name: "file_write".into(),
        category: "Filesystem".into(),
        description: "Write a file".into(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            },
            "required": ["path", "content"]
        }),
    }
}

fn make_input() -> VerificationInput {
    VerificationInput::new(
        "Write hello to /tmp/out.txt".into(),
        make_response("Writing the requested file.", vec![make_tool_call()]),
        vec![make_tool_info()],
    )
}

fn build_circuit_check() -> CircuitCheck {
    let check = CircuitCheck::new();
    let bytes = to_bytes(&build_not_circuit());
    check
        .register(
            arkavo_critic::PolicyId::new("block_code_without_tools"),
            &bytes,
            vec![FeatureId::HasToolCalls],
        )
        .unwrap();
    check
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
}

/// Collect `SAMPLES` durations and print p50/p95 in microseconds.
fn report<R>(stage: &str, mut f: impl FnMut() -> R) {
    // Warmup
    for _ in 0..20 {
        f();
    }
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        f();
        samples.push(start.elapsed().as_nanos() as f64 / 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    let p50 = samples[SAMPLES / 2];
    let p95 = samples[(SAMPLES as f64 * 0.95).ceil() as usize - 1];
    println!("GATE_STAGE {stage} p50_us={p50:.2} p95_us={p95:.2}");
}

struct GateStages {
    moderator: PreflightModerator,
    tracker: BudgetTracker,
    circuit: CircuitCheck,
    schema: SchemaCheck,
    policy: PolicyCheck,
    pipeline: arkavo_critic::CriticPipeline,
    input: VerificationInput,
    prompt: &'static str,
    signing_key: SigningKey,
    signature: ed25519_dalek::Signature,
}

async fn build_stages() -> GateStages {
    let tracker = BudgetTracker::new(BudgetConfig::default()).await.unwrap();
    let signing_key = SigningKey::from_bytes(&[7u8; 32]);
    let signature = signing_key.sign(b"dispatch-gate-baseline");
    GateStages {
        moderator: build_moderator(),
        tracker,
        circuit: build_circuit_check(),
        schema: SchemaCheck::new(),
        policy: PolicyCheck::with_security_defaults(),
        pipeline: arkavo_critic::default_pipeline(),
        input: make_input(),
        prompt: "Write hello to /tmp/out.txt",
        signing_key,
        signature,
    }
}

fn bench_gate_stages(c: &mut Criterion) {
    let rt = runtime();
    let stages = rt.block_on(build_stages());
    let cost = TokenCost::from_dollars(0.001);
    let msg = b"dispatch-gate-baseline";

    report("preflight_moderator_3_policies", || {
        black_box(stages.moderator.check(black_box(stages.prompt)));
    });
    report("budget_can_afford", || {
        rt.block_on(async {
            black_box(
                stages
                    .tracker
                    .can_afford(black_box("agent-bench"), black_box(cost))
                    .await,
            )
        })
    });
    report("critic_circuit_check", || {
        rt.block_on(async { black_box(stages.circuit.verify(black_box(&stages.input)).await) })
    });
    report("critic_schema_check", || {
        rt.block_on(async { black_box(stages.schema.verify(black_box(&stages.input)).await) })
    });
    report("critic_policy_check", || {
        rt.block_on(async { black_box(stages.policy.verify(black_box(&stages.input)).await) })
    });
    report("critic_default_pipeline", || {
        rt.block_on(async { black_box(stages.pipeline.verify(black_box(&stages.input)).await) })
    });
    report("ed25519_verify", || {
        black_box(
            stages
                .signing_key
                .verifying_key()
                .verify(black_box(msg), black_box(&stages.signature)),
        )
    });
    report("full_policy_sequence", || {
        black_box(stages.moderator.check(black_box(stages.prompt)));
        rt.block_on(async {
            let _ = black_box(
                stages
                    .tracker
                    .can_afford(black_box("agent-bench"), black_box(cost))
                    .await,
            );
            black_box(stages.pipeline.verify(black_box(&stages.input)).await)
        })
    });

    let mut group = c.benchmark_group("dispatch_gate");

    group.bench_function("preflight_moderator_3_policies", |b| {
        b.iter(|| stages.moderator.check(black_box(stages.prompt)))
    });

    group.bench_function("budget_can_afford", |b| {
        b.iter(|| rt.block_on(async { stages.tracker.can_afford("agent-bench", cost).await }))
    });

    group.bench_function("critic_circuit_check", |b| {
        b.iter(|| rt.block_on(async { stages.circuit.verify(&stages.input).await }))
    });

    group.bench_function("critic_schema_check", |b| {
        b.iter(|| rt.block_on(async { stages.schema.verify(&stages.input).await }))
    });

    group.bench_function("critic_policy_check", |b| {
        b.iter(|| rt.block_on(async { stages.policy.verify(&stages.input).await }))
    });

    group.bench_function("critic_default_pipeline", |b| {
        b.iter(|| rt.block_on(async { stages.pipeline.verify(&stages.input).await }))
    });

    group.bench_function("ed25519_verify", |b| {
        b.iter(|| {
            stages
                .signing_key
                .verifying_key()
                .verify(black_box(msg), &stages.signature)
        })
    });

    group.bench_function("full_policy_sequence", |b| {
        b.iter(|| {
            let _ = black_box(stages.moderator.check(black_box(stages.prompt)));
            rt.block_on(async {
                stages.tracker.can_afford("agent-bench", cost).await.ok();
                stages.pipeline.verify(&stages.input).await
            })
        })
    });

    group.finish();
}

criterion_group!(benches, bench_gate_stages);
criterion_main!(benches);
