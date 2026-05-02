//! End-to-end integration tests for the EvoFabric AST pipeline.
//!
//! Exercises the full pipeline: construct OpBundle → parse source → apply ops
//! → render → verify the rendered output is valid Rust that could compile.

use arkavo_evofabric::{OpBundle, RustOp, render_file};
use arkavo_test_macros::spec;

/// Realistic source file used as the target for integration tests.
/// Mirrors the structure of a real crate module with structs, impls, and free fns.
const SAMPLE_SOURCE: &str = r##"
use std::collections::HashMap;

/// Configuration for a processing pipeline.
pub struct PipelineConfig {
    pub name: String,
    pub max_retries: u32,
    pub timeout_ms: u64,
    pub tags: HashMap<String, String>,
}

impl PipelineConfig {
    pub fn new(name: String) -> Self {
        Self {
            name,
            max_retries: 3,
            timeout_ms: 5000,
            tags: HashMap::new(),
        }
    }

    pub fn with_timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    pub fn is_valid(&self) -> bool {
        !self.name.is_empty() && self.timeout_ms > 0
    }
}

/// Check if a model name indicates a sub-1B parameter model.
fn is_small_model(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("0.6b")
        || lower.contains("0.8b")
        || lower.contains("270m")
        || lower.contains("500m")
}

fn process(cfg: &PipelineConfig) -> bool {
    cfg.is_valid() && cfg.max_retries > 0
}
"##;

#[spec("EVO-001")]
#[test]
fn end_to_end_add_inline_to_free_function() {
    let bundle_json = r##"{
        "id": "00000000-0000-0000-0000-000000000000",
        "originator": "evofabric-agent",
        "target_file": "crates/test-crate/src/lib.rs",
        "ops": [
            {
                "op": "AddAttribute",
                "scope": { "kind": "Function", "name": "is_small_model" },
                "attribute": "#[inline]"
            }
        ],
        "rationale": "Add inline hint to small function",
        "created_at": "2026-01-01T00:00:00Z",
        "content_hash": "",
        "status": { "state": "Proposed" }
    }"##;

    let bundle = OpBundle::from_json(bundle_json).expect("parse OpBundle");
    assert_eq!(bundle.ops.len(), 1);

    let result = RustOp::apply_bundle(SAMPLE_SOURCE, &bundle).expect("apply bundle");
    let rendered = render_file(&result);

    assert!(rendered.contains("#[inline]"));
    assert!(rendered.contains("fn is_small_model"));

    // Verify the rendered output re-parses successfully
    RustOp::parse(&rendered).expect("re-parse rendered output");
}

#[spec("EVO-001")]
#[test]
fn end_to_end_replace_method_body() {
    let bundle_json = r##"{
        "id": "00000000-0000-0000-0000-000000000000",
        "originator": "evofabric-agent",
        "target_file": "crates/test-crate/src/lib.rs",
        "ops": [
            {
                "op": "ReplaceFnBody",
                "scope": {
                    "kind": "Function",
                    "name": "is_valid",
                    "within": { "kind": "ImplBlock", "type_name": "PipelineConfig" }
                },
                "new_body": "{ !self.name.is_empty() && self.timeout_ms > 0 && self.max_retries <= 10 }"
            }
        ],
        "rationale": "Add upper bound on max_retries",
        "created_at": "2026-01-01T00:00:00Z",
        "content_hash": "",
        "status": { "state": "Proposed" }
    }"##;

    let bundle = OpBundle::from_json(bundle_json).expect("parse OpBundle");
    let result = RustOp::apply_bundle(SAMPLE_SOURCE, &bundle).expect("apply bundle");
    let rendered = render_file(&result);

    assert!(rendered.contains("max_retries <= 10"));
    RustOp::parse(&rendered).expect("re-parse rendered output");
}

#[spec("EVO-001")]
#[test]
fn end_to_end_multi_op_bundle() {
    let bundle_json = r##"{
        "id": "00000000-0000-0000-0000-000000000000",
        "originator": "evofabric-agent",
        "target_file": "crates/test-crate/src/lib.rs",
        "ops": [
            {
                "op": "AddUse",
                "path": "std::time::Duration"
            },
            {
                "op": "AddAttribute",
                "scope": { "kind": "Function", "name": "process" },
                "attribute": "#[must_use]"
            },
            {
                "op": "ReplaceFnBody",
                "scope": { "kind": "Function", "name": "process" },
                "new_body": "{ cfg.is_valid() && cfg.max_retries > 0 && cfg.timeout_ms < 60_000 }"
            }
        ],
        "rationale": "Harden process with timeout upper bound",
        "created_at": "2026-01-01T00:00:00Z",
        "content_hash": "",
        "status": { "state": "Proposed" }
    }"##;

    let bundle = OpBundle::from_json(bundle_json).expect("parse OpBundle");
    assert_eq!(bundle.ops.len(), 3);

    let result = RustOp::apply_bundle(SAMPLE_SOURCE, &bundle).expect("apply bundle");
    let rendered = render_file(&result);

    assert!(rendered.contains("use std::time::Duration;"));
    assert!(rendered.contains("#[must_use]"));
    assert!(rendered.contains("timeout_ms < 60_000"));

    RustOp::parse(&rendered).expect("re-parse rendered output");
}

#[spec("EVO-001")]
#[test]
fn end_to_end_insert_new_method() {
    let bundle_json = r##"{
        "id": "00000000-0000-0000-0000-000000000000",
        "originator": "evofabric-agent",
        "target_file": "crates/test-crate/src/lib.rs",
        "ops": [
            {
                "op": "InsertAfter",
                "scope": { "kind": "ImplBlock", "type_name": "PipelineConfig" },
                "new_item": "impl PipelineConfig { pub fn tag_count(&self) -> usize { self.tags.len() } }"
            }
        ],
        "rationale": "Add convenience method for tag count",
        "created_at": "2026-01-01T00:00:00Z",
        "content_hash": "",
        "status": { "state": "Proposed" }
    }"##;

    let bundle = OpBundle::from_json(bundle_json).expect("parse OpBundle");
    let result = RustOp::apply_bundle(SAMPLE_SOURCE, &bundle).expect("apply bundle");
    let rendered = render_file(&result);

    assert!(rendered.contains("tag_count"));
    assert!(rendered.contains("self.tags.len()"));

    RustOp::parse(&rendered).expect("re-parse rendered output");
}

#[spec("EVO-001")]
#[test]
fn end_to_end_from_json_with_markdown_fences() {
    let llm_output = r##"```json
{
    "id": "00000000-0000-0000-0000-000000000000",
    "originator": "evofabric-agent",
    "target_file": "crates/test-crate/src/lib.rs",
    "ops": [
        {
            "op": "AddAttribute",
            "scope": { "kind": "TypeDef", "name": "PipelineConfig" },
            "attribute": "#[derive(Clone)]"
        }
    ],
    "rationale": "Add Clone",
    "created_at": "2026-01-01T00:00:00Z",
    "content_hash": "",
    "status": { "state": "Proposed" }
}
```"##;

    let bundle = OpBundle::from_json(llm_output).expect("parse with markdown fences");
    let result = RustOp::apply_bundle(SAMPLE_SOURCE, &bundle).expect("apply bundle");
    let rendered = render_file(&result);

    assert!(rendered.contains("#[derive(Clone)]"));
    RustOp::parse(&rendered).expect("re-parse rendered output");
}

#[spec("EVO-001")]
#[test]
fn end_to_end_bundle_rejection_on_invalid_body() {
    let bundle_json = r##"{
        "id": "00000000-0000-0000-0000-000000000000",
        "originator": "evofabric-agent",
        "target_file": "crates/test-crate/src/lib.rs",
        "ops": [
            {
                "op": "ReplaceFnBody",
                "scope": { "kind": "Function", "name": "is_small_model" },
                "new_body": "{ let x = @@@invalid@@@ }"
            }
        ],
        "rationale": "This should fail",
        "created_at": "2026-01-01T00:00:00Z",
        "content_hash": "",
        "status": { "state": "Proposed" }
    }"##;

    let bundle = OpBundle::from_json(bundle_json).expect("parse OpBundle");
    let err = RustOp::apply_bundle(SAMPLE_SOURCE, &bundle).unwrap_err();
    assert!(
        err.to_string().contains("parse error"),
        "Expected parse error, got: {err}"
    );
}

#[spec("EVO-001")]
#[test]
fn end_to_end_bundle_rejection_on_missing_target() {
    let bundle_json = r##"{
        "id": "00000000-0000-0000-0000-000000000000",
        "originator": "evofabric-agent",
        "target_file": "crates/test-crate/src/lib.rs",
        "ops": [
            {
                "op": "Remove",
                "scope": { "kind": "Function", "name": "nonexistent_fn" }
            }
        ],
        "rationale": "Target does not exist",
        "created_at": "2026-01-01T00:00:00Z",
        "content_hash": "",
        "status": { "state": "Proposed" }
    }"##;

    let bundle = OpBundle::from_json(bundle_json).expect("parse OpBundle");
    let err = RustOp::apply_bundle(SAMPLE_SOURCE, &bundle).unwrap_err();
    assert!(
        err.to_string().contains("not found"),
        "Expected target not found error, got: {err}"
    );
}

#[spec("EVO-001")]
#[test]
fn end_to_end_replace_item_whole_function() {
    let bundle_json = r##"{
        "id": "00000000-0000-0000-0000-000000000000",
        "originator": "evofabric-agent",
        "target_file": "crates/test-crate/src/lib.rs",
        "ops": [
            {
                "op": "ReplaceItem",
                "scope": { "kind": "Function", "name": "is_small_model" },
                "new_item": "fn is_small_model(name: &str) -> bool { name.len() < 4 }"
            }
        ],
        "rationale": "Simplify model size check",
        "created_at": "2026-01-01T00:00:00Z",
        "content_hash": "",
        "status": { "state": "Proposed" }
    }"##;

    let bundle = OpBundle::from_json(bundle_json).expect("parse OpBundle");
    let result = RustOp::apply_bundle(SAMPLE_SOURCE, &bundle).expect("apply bundle");
    let rendered = render_file(&result);

    assert!(rendered.contains("name.len() < 4"));
    assert!(!rendered.contains("0.6b"));

    RustOp::parse(&rendered).expect("re-parse rendered output");
}
