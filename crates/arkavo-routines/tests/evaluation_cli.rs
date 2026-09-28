use std::process::Command;

use serde_json::json;

fn measurements(execution: u64, learning: u64) -> String {
    [1, 2].into_iter().flat_map(|seed| {
        ["cold", "warm", "changed_environment"].map(move |phase| {
            json!({"task_id":"accounting-fixture","phase":phase,"order_seed":seed,"success":true,
                "execution_tokens":execution,"learning_tokens":learning,"verification_tokens":0,
                "prompt_tokens":execution,"cached_prompt_tokens":0,"repeated_actions":0,
                "primitive_calls":3,"latency_ms":10,"max_routing_us":1000,"policy_violations":0}).to_string()
        })
    }).collect::<Vec<_>>().join("\n")
}

#[test]
fn cli_includes_learning_cost_and_reports_gate_failure() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.jsonl");
    let candidate = dir.path().join("candidate.jsonl");
    std::fs::write(&base, measurements(100, 0)).unwrap();
    std::fs::write(&candidate, measurements(60, 10)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_routine-evaluate"))
        .arg(&base)
        .arg(&candidate)
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["passes_pilot_gate"], true);
    assert_eq!(report["candidate"]["tokens_per_success"], 70.0);
    std::fs::write(&candidate, measurements(60, 50)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_routine-evaluate"))
        .arg(&base)
        .arg(&candidate)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["passes_pilot_gate"], false);
}

#[test]
fn cli_rejects_missing_files_malformed_rows_and_missing_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.jsonl");
    let output = Command::new(env!("CARGO_BIN_EXE_routine-evaluate"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let output = Command::new(env!("CARGO_BIN_EXE_routine-evaluate"))
        .arg(&path)
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    std::fs::write(&path, "{}").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_routine-evaluate"))
        .arg(&path)
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
}
