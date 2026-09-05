//! Parity test: generation with NGRAM spec decoding must produce identical
//! tokens to generation without spec, at a fixed seed and temperature=0.0.
//! Any divergence is a correctness bug in the spec integration.
//!
//! Gated on `ARKAVO_TEST_MODEL` because it needs a real GGUF model. CI has
//! no model so this is `#[ignore]`d by default; run locally with:
//!
//!   ARKAVO_TEST_MODEL=$HOME/.arkavo/models/qwen3.5-9b-q4_k_m.gguf \
//!     cargo test -p arkavo-llm --test spec_parity_test -- --ignored --nocapture

#![cfg(all(feature = "llama-cpp", not(target_env = "musl")))]

use arkavo_llm::llamacpp_provider::{LlamaCppProvider, SamplingConfig};
use arkavo_llm::provider::Provider;
use arkavo_llm::{Message, Role};

fn make_provider(model_path: &str, use_spec: bool) -> LlamaCppProvider {
    make_provider_with_max_tokens(model_path, use_spec, 80)
}

fn make_provider_with_max_tokens(
    model_path: &str,
    use_spec: bool,
    max_tokens: u32,
) -> LlamaCppProvider {
    let config = SamplingConfig {
        temperature: 0.0,
        seed: 42,
        max_tokens,
        debug: false,
        use_spec_decoding: use_spec,
        ..Default::default()
    };
    LlamaCppProvider::new_with_config(
        "spec-parity-test".to_string(),
        model_path.to_string(),
        None,
        config,
    )
    .expect("Failed to load model for parity test")
}

fn make_messages() -> Vec<Message> {
    vec![Message {
        response_items: Vec::new(),
        role: Role::User,
        content: "Write a JSON object with three keys: name, version, type. Output only the JSON."
            .to_string(),
        images: None,
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::new(),
    }]
}

/// Build a user prompt that tokenizes to roughly `approx_tokens`. Uses
/// a repeated phrase so the content has natural ngram structure and we
/// don't accidentally probe just a tokenizer pathological case.
fn make_long_messages(approx_tokens: usize) -> Vec<Message> {
    let phrase = "The colony survival problem requires careful planning of food, shelter, \
                  research, and defense. Each colonist has needs that must be balanced \
                  against available resources. Weather, threats, and morale interact in \
                  complex ways across the simulation. ";
    let chars_needed = approx_tokens * 4;
    let mut s = String::with_capacity(chars_needed + phrase.len());
    while s.len() < chars_needed {
        s.push_str(phrase);
    }
    s.push_str(
        "\n\nGiven the above context, write a single short JSON object with keys \
         `summary` (one sentence) and `priority` (low|med|high). Output only the JSON.",
    );
    vec![Message {
        response_items: Vec::new(),
        role: Role::User,
        content: s,
        images: None,
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::new(),
    }]
}

#[tokio::test]
#[ignore = "requires local GGUF model; opt-in via ARKAVO_TEST_MODEL and --ignored"]
async fn ngram_spec_matches_baseline_output() {
    let model_path = match std::env::var("ARKAVO_TEST_MODEL") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("Skipping: set ARKAVO_TEST_MODEL=/path/to/model.gguf to run");
            return;
        }
    };

    let baseline_provider = make_provider(&model_path, false);
    let spec_provider = make_provider(&model_path, true);

    let baseline = baseline_provider
        .complete(make_messages())
        .await
        .expect("baseline completion failed");

    let spec = spec_provider
        .complete(make_messages())
        .await
        .expect("spec completion failed");

    assert_eq!(
        baseline, spec,
        "spec decoding diverged from baseline output:\n  baseline: {baseline:?}\n  spec: {spec:?}",
    );
}

/// Sanity check that the spec path actually drafted tokens on a
/// structured-output prompt. If `n_draft == 0` the parity test above
/// silently passes even with a broken integration, so this guards
/// against that footgun.
#[tokio::test]
#[ignore = "requires local GGUF model; opt-in via ARKAVO_TEST_MODEL and --ignored"]
async fn ngram_spec_produces_drafts_on_structured_prompt() {
    let model_path = match std::env::var("ARKAVO_TEST_MODEL") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("Skipping: set ARKAVO_TEST_MODEL=/path/to/model.gguf to run");
            return;
        }
    };

    let spec_provider = make_provider(&model_path, true);
    let response = spec_provider
        .complete_with_tools(make_messages(), None, None)
        .await
        .expect("spec completion failed");

    let timing = response
        .inference_timing
        .expect("spec path must populate inference_timing");

    // Spec is intentionally bypassed for M-RoPE models (Qwen3-VL/3.6, etc.)
    // because rolling back unaccepted drafts violates b9292's M-RoPE position
    // monotonicity check. Treat that as an acceptable skip rather than a
    // failure — the bypass reason is the proof that the gate fired.
    if let Some(reason) = timing.spec_bypassed.as_deref() {
        eprintln!("Spec bypassed for this model: {reason} — skipping draft assertion");
        return;
    }

    let n_draft = timing.n_draft.unwrap_or(0);
    assert!(
        n_draft > 0,
        "spec should have drafted on structured output prompt, got n_draft={n_draft}; \
         n_accepted={:?}",
        timing.n_accepted,
    );
}

/// Repro for the b9292 "GPU fault (DecodeFailure)" symptom: long prompts
/// (~5K tokens) crash mid-generation with `llama_decode failed with code: -1`.
/// Code -1 is "invalid input batch" per llama.h, not a GPU error.
///
/// This test runs both with and without spec decoding to isolate whether the
/// spec verification batch is the trigger or whether the bug also fires on
/// the plain single-token decode path inside `spec.rs`.
#[tokio::test]
#[ignore = "requires local GGUF model; opt-in via ARKAVO_TEST_MODEL and --ignored"]
async fn long_prompt_does_not_emit_invalid_batch() {
    let model_path = match std::env::var("ARKAVO_TEST_MODEL") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("Skipping: set ARKAVO_TEST_MODEL=/path/to/model.gguf to run");
            return;
        }
    };

    let approx_tokens: usize = std::env::var("ARKAVO_TEST_PROMPT_TOKENS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5000);

    // Use complete_with_tools and a Qwen3-style tool schema so the grammar
    // wrapper activates — the swarm-side failures all happen with tools
    // (and PEG/grammar constraints) attached, never on a plain text prompt.
    let tools = serde_json::json!([
        {
            "type": "function",
            "function": {
                "name": "observe",
                "description": "Observe the current colony state",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "Include": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "step",
                "description": "Execute an action in the game",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "AgentId": { "type": "string" },
                        "Action": { "type": "object" },
                        "Ticks": { "type": "integer" }
                    },
                    "required": ["AgentId", "Action"]
                }
            }
        }
    ]);

    eprintln!("=== Spec ON, ~{approx_tokens}-token prompt with tools ===");
    let spec_provider = make_provider(&model_path, true);
    let spec_result = spec_provider
        .complete_with_tools(
            make_long_messages(approx_tokens),
            Some(tools.clone()),
            Some(512),
        )
        .await;
    let spec_result = match spec_result {
        Ok(r) => {
            eprintln!(
                "  spec OK: {} chars, {} tool_calls, timing={:?}",
                r.content.len(),
                r.tool_calls.len(),
                r.inference_timing
                    .as_ref()
                    .map(|t| (t.n_draft, t.n_accepted, &t.spec_bypassed))
            );
            Ok(r.content)
        }
        Err(e) => {
            eprintln!("  spec ERROR: {e}");
            Err(e)
        }
    };

    eprintln!("=== Spec OFF, ~{approx_tokens}-token prompt with tools ===");
    let baseline_provider = make_provider(&model_path, false);
    let baseline_result = baseline_provider
        .complete_with_tools(make_long_messages(approx_tokens), Some(tools), Some(512))
        .await;
    let baseline_result = match baseline_result {
        Ok(r) => {
            eprintln!(
                "  baseline OK: {} chars, {} tool_calls",
                r.content.len(),
                r.tool_calls.len()
            );
            Ok(r.content)
        }
        Err(e) => {
            eprintln!("  baseline ERROR: {e}");
            Err(e)
        }
    };

    let spec_ok = spec_result.is_ok();
    let baseline_ok = baseline_result.is_ok();
    let spec_invalid_batch = spec_result
        .as_ref()
        .err()
        .map(|e| e.to_string().contains("code: -1"))
        .unwrap_or(false);
    let baseline_invalid_batch = baseline_result
        .as_ref()
        .err()
        .map(|e| e.to_string().contains("code: -1"))
        .unwrap_or(false);

    // Report — never silently pass on -1. Fail loudly so it shows up in CI logs.
    assert!(
        !spec_invalid_batch,
        "spec path emitted llama_decode code -1 (invalid input batch) on {approx_tokens}-token prompt"
    );
    assert!(
        !baseline_invalid_batch,
        "non-spec path emitted llama_decode code -1 (invalid input batch) on {approx_tokens}-token prompt"
    );
    assert!(spec_ok, "spec completion failed: {:?}", spec_result.err());
    assert!(
        baseline_ok,
        "baseline completion failed: {:?}",
        baseline_result.err()
    );
}

/// Regression for the spec-batch overflow at the end of the context window.
/// When the planner pushes `pos` to within N_DRAFT_MAX of safe_ctx, the spec
/// verification batch `[pos..pos+drafts.len()]` overflowed the allocated KV
/// slots and `llama_decode` returned code 1 ("could not find a KV slot"),
/// which the GPU breaker then retried pointlessly with the same oversize
/// batch. The fix clamps `drafts.len()` to `safe_ctx - pos - 1` per cycle.
///
/// To exercise the boundary we want a prompt large enough that one full spec
/// batch (`N_DRAFT_MAX = 8` tokens) plus a single generation step would land
/// past `safe_ctx`. For `qwen3.6-35b-a3b` and `gemma-4-26b-a4b`, safe_ctx is
/// `min(trained_ctx / 4, 16384) = 16384`, so we target ~15800 prompt tokens
/// + 600 max_tokens to push generation right against the boundary.
#[tokio::test]
#[ignore = "requires local GGUF model; opt-in via ARKAVO_TEST_MODEL and --ignored"]
async fn near_context_limit_does_not_emit_kv_slot_failure() {
    let model_path = match std::env::var("ARKAVO_TEST_MODEL") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("Skipping: set ARKAVO_TEST_MODEL=/path/to/model.gguf to run");
            return;
        }
    };

    let approx_tokens: usize = std::env::var("ARKAVO_TEST_PROMPT_TOKENS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(15800);
    let max_tokens: u32 = std::env::var("ARKAVO_TEST_MAX_TOKENS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);

    eprintln!("=== Spec ON, ~{approx_tokens}-token prompt + {max_tokens} max_tokens ===");
    let provider = make_provider_with_max_tokens(&model_path, true, max_tokens);
    let result = provider.complete(make_long_messages(approx_tokens)).await;
    match &result {
        Ok(s) => eprintln!("  OK: {} chars generated", s.len()),
        Err(e) => eprintln!("  ERROR: {e}"),
    }

    let saw_kv_slot_failure = result
        .as_ref()
        .err()
        .map(|e| {
            let msg = e.to_string();
            msg.contains("code: 1") || msg.contains("could not find a KV slot")
        })
        .unwrap_or(false);

    assert!(
        !saw_kv_slot_failure,
        "spec batch overflowed safe_ctx near the end of the context window — \
         clamp logic in spec.rs failed to cap drafts.len() to (safe_ctx - pos - 1)"
    );
    assert!(
        result.is_ok(),
        "near-context-limit completion failed: {:?}",
        result.err()
    );
}
