//! Verifies the committed sample contract (loaded by `arkavo eval run`)
//! deserializes into `EvalContract`. The CLI binary itself cannot be built in
//! every environment (it transitively pulls the vendored llama.cpp build), so
//! this test pins the one runtime risk of the CLI wrapper: contract loading.

use arkavo_eval::contract::EvalContract;

#[test]
fn sample_contract_fixture_deserializes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample_contract.json");
    let json = std::fs::read_to_string(path).expect("fixture present");
    let c: EvalContract = serde_json::from_str(&json).expect("fixture matches EvalContract schema");

    assert_eq!(c.task_kind, "model_eval");
    assert_eq!(c.model.name, "gemma-4-12b");
    assert_eq!(c.prompts.len(), 1);
    assert_eq!(c.prompts[0].id, "capital");
    assert_eq!(c.baseline.commit.as_deref(), Some("local"));
    assert_eq!(c.preconditions, vec!["weights_present", "baseline_present"]);
    assert_eq!(c.on_precondition_unmet, "refuse");
}
