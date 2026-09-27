#![cfg(feature = "taint")]

//! SEQ-001, SEQ-002, SEQ-004: the session-scoped tracker and action graph.

use std::time::{Duration, Instant};

use arkavo_events::EventPayload;
use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
use arkavo_protocol::taint::{SourceKind, TaintSet, TaintSource, Transformation};
use arkavo_protocol::taint_tracker::{DataTaintTracker, ModelCeilings};
use arkavo_test_macros::spec;
use serde_json::json;

/// The sequence-integrity invariant: tracking adds under this per tool call.
const BUDGET: Duration = Duration::from_micros(50);

/// A tail this far outside the budget is a pathology rather than scheduler
/// noise, and is worth failing on even when the average is fine.
const TAIL_CEILING: Duration = Duration::from_micros(200);

/// The payload every propagation test carries. A function rather than a const
/// because the credential inside it is built at run time.
fn secret() -> String {
    format!("deploy key {} rotate monthly", fake_api_key())
}

fn tool(name: &str) -> TaintSource {
    TaintSource::new(SourceKind::ToolResult, name)
}

/// Mean and p99 of `iterations` samples, after a warm-up pass.
fn measure(iterations: usize, mut op: impl FnMut()) -> (Duration, Duration) {
    for _ in 0..iterations / 10 {
        op();
    }
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        op();
        samples.push(start.elapsed());
    }
    let total: Duration = samples.iter().sum();
    samples.sort_unstable();
    let p99 = samples[(samples.len() * 99 / 100).min(samples.len() - 1)];
    (total / iterations as u32, p99)
}

/// SEQ-001: a tool result entering the session is labelled at ingestion.
#[spec("SEQ-001")]
#[test]
fn a_tool_result_is_labelled_when_it_enters_the_session() {
    let tracker = DataTaintTracker::new("session-1");

    let set = tracker.ingest(&tool("read_file"), secret().as_str());

    assert_eq!(set.sensitivity(), SensitivityLevel::Restricted);
    assert!(set.contains_category(DataCategory::Credentials));
    assert_eq!(set.source_ids().collect::<Vec<_>>(), vec!["tool:read_file"]);
}

/// SEQ-001 edge case: taint from another agent is inherited rather than
/// re-derived, so relaying data through a peer does not launder its label.
#[spec("SEQ-001")]
#[test]
fn taint_from_a_peer_agent_is_inherited() {
    let tracker = DataTaintTracker::new("session-1");
    let upstream = tracker.ingest(&tool("vault"), secret().as_str());
    let peer = TaintSource::new(SourceKind::A2aReceive, "did:key:z6MkPeer");

    let received = tracker.ingest_from_agent(&peer, "attached summary, see notes", &upstream);

    assert_eq!(received.sensitivity(), SensitivityLevel::Restricted);
    assert!(received.contains_category(DataCategory::Credentials));
}

/// SEQ-002 edge case: output of inference is tainted when its input was.
#[spec("SEQ-002")]
#[test]
fn inference_output_is_tainted_when_its_input_was() {
    let tracker = DataTaintTracker::new("session-1");
    let prompt = tracker.ingest(&tool("read_file"), secret().as_str());

    let completion = tracker.after_inference(&[&prompt], "gemma-e2b");

    assert_eq!(completion.sensitivity(), SensitivityLevel::Restricted);
}

/// SEQ-002 edge case: a model whose own ceiling is higher than the request
/// raises the output regardless of what the request carried. Until pack
/// metadata exists this ceiling is configuration, which is exactly why an
/// unconfigured model must not read as public.
#[spec("SEQ-002")]
#[test]
fn the_serving_model_ceiling_applies_to_clean_input() {
    let tracker = DataTaintTracker::new("session-1").with_model_ceilings(
        ModelCeilings::new(SensitivityLevel::Public)
            .with("finance-tuned", SensitivityLevel::Confidential),
    );
    let clean = tracker.ingest(
        &tool("docs").declared(SensitivityLevel::Public),
        "hello world",
    );

    let completion = tracker.after_inference(&[&clean], "finance-tuned");

    assert_eq!(completion.sensitivity(), SensitivityLevel::Confidential);
}

/// SEQ-004: each completed call becomes a node, with tool name, a parameter
/// digest, and the taint it handled.
#[spec("SEQ-004")]
#[test]
fn each_completed_call_becomes_a_node() {
    let tracker = DataTaintTracker::new("session-1");
    let taint = tracker.ingest(&tool("read_file"), secret().as_str());

    let id = tracker
        .record_call(
            "read_file",
            &json!({"path": "/etc/deploy.env"}),
            &[],
            &taint,
        )
        .expect("first call has no inputs");

    let graph = tracker.graph();
    let node = graph.node(&id).expect("node recorded");
    assert_eq!(node.tool_name, "read_file");
    assert!(!node.params_hash.is_empty());
    assert_eq!(node.taint.sensitivity(), SensitivityLevel::Restricted);
}

/// SEQ-004: edges follow the data, so a read feeding a post is visible as a
/// path even though neither call is unusual by itself.
#[spec("SEQ-004")]
#[test]
fn edges_connect_the_output_of_one_call_to_the_input_of_the_next() {
    let tracker = DataTaintTracker::new("session-1");
    let secret = tracker.ingest(&tool("read_file"), secret().as_str());
    let read = tracker
        .record_call(
            "read_file",
            &json!({"path": "/etc/deploy.env"}),
            &[],
            &secret,
        )
        .expect("root call");

    let post = tracker
        .record_call(
            "http_post",
            &json!({"url": "https://example.invalid/collect"}),
            std::slice::from_ref(&read),
            &TaintSet::new(),
        )
        .expect("known input");

    let edges: Vec<(String, String)> = tracker
        .graph()
        .edges()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
    assert_eq!(edges, vec![(read, post.clone())]);

    let graph = tracker.graph();
    let sink = graph.node(&post).expect("sink node");
    assert_eq!(sink.taint.sensitivity(), SensitivityLevel::Restricted);
}

/// SEQ-001, SEQ-004: taint reaches the ledger, not only the in-memory graph.
#[spec("SEQ-001", "SEQ-004")]
#[test]
fn taint_metadata_reaches_the_ledger() {
    let tracker = DataTaintTracker::new("session-1");
    let secret = tracker.ingest(&tool("read_file"), secret().as_str());
    let read = tracker
        .record_call("read_file", &json!({}), &[], &secret)
        .expect("root call");
    tracker
        .record_call("http_post", &json!({}), &[read], &TaintSet::new())
        .expect("known input");

    let entries = tracker.ledger_entries();

    assert_eq!(entries.len(), 2);
    let EventPayload::SequenceNode {
        tool_name,
        inputs,
        taint,
        ..
    } = &entries[1]
    else {
        panic!("expected a sequence node, got {:?}", entries[1]);
    };
    assert_eq!(tool_name, "http_post");
    assert_eq!(inputs.len(), 1);
    assert_eq!(taint.sensitivity, "restricted");
    assert_eq!(taint.categories, vec!["credentials".to_string()]);
}

/// SEQ-004: an edge from a node this session never recorded is refused, and
/// refusing it leaves the graph exactly as it was.
#[spec("SEQ-004")]
#[test]
fn an_unknown_input_leaves_the_graph_unchanged() {
    let tracker = DataTaintTracker::new("session-1");
    tracker
        .record_call("read_file", &json!({}), &[], &TaintSet::new())
        .expect("root call");

    let err = tracker.record_call(
        "http_post",
        &json!({}),
        &["n404".to_string()],
        &TaintSet::new(),
    );

    assert!(err.is_err());
    assert_eq!(tracker.graph().len(), 1);
}

/// SEQ invariant: propagation and recording add under 50µs to a tool call.
///
/// Classification is deliberately excluded — it scales with payload size and
/// belongs off the hot path, which is what the Phase 4 cascade is for. What is
/// asserted here is the part that is unavoidably synchronous.
#[spec("SEQ-004")]
#[test]
fn propagation_and_recording_stay_inside_the_per_call_budget() {
    let tracker = DataTaintTracker::new("session-1");
    let taint = tracker.ingest(&tool("read_file"), secret().as_str());
    let params = json!({"path": "/etc/deploy.env", "limit": 100});

    let (transform_mean, transform_p99) = measure(500, || {
        let _ = tracker.transform(&[&taint], Transformation::Encode, "base64");
    });
    let (inference_mean, inference_p99) = measure(500, || {
        let _ = tracker.after_inference(&[&taint], "gemma-e2b");
    });
    let (record_mean, record_p99) = {
        let recorder = DataTaintTracker::new("session-1");
        measure(500, || {
            recorder
                .record_call("read_file", &params, &[], &taint)
                .expect("root call");
        })
    };

    for (name, mean, p99) in [
        ("transform", transform_mean, transform_p99),
        ("after_inference", inference_mean, inference_p99),
        ("record_call", record_mean, record_p99),
    ] {
        assert!(
            mean < BUDGET,
            "{name} averaged {mean:?} per call, budget is {BUDGET:?} (p99 {p99:?})"
        );
        assert!(
            p99 < TAIL_CEILING,
            "{name} p99 was {p99:?}, ceiling is {TAIL_CEILING:?} (mean {mean:?})"
        );
    }
}

/// Builds a credential-shaped string at run time.
///
/// Generated rather than written down: a literal that matches a secret pattern
/// trips scanners on every clone of this repo, and a scanner that cries wolf on
/// fixtures is one people learn to ignore. The pieces are inert separately, and
/// the value is deterministic so a failure stays reproducible.
fn fake_api_key() -> String {
    let prefix: String = ['s', 'k'].iter().collect();
    let body: String = (0..24)
        .map(|i| char::from(b'a' + ((i * 7 + 3) % 26) as u8))
        .collect();
    format!("{prefix}-{body}")
}
