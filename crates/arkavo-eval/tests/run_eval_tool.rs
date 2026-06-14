#![cfg(feature = "mcp-tool")]
// #[tokio::test] expands to Runtime::block_on, which .clippy.toml disallows in lib/bin code.
#![allow(clippy::disallowed_methods)]

use arkavo_eval::tool::{EvalState, ModelResolver, RunEvalTool};
use arkavo_mcp_tools::server::Tool;
use serde_json::json;
use std::sync::Arc;

fn find_model() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let base = format!("{home}/.cache/huggingface/hub/models--unsloth--gemma-3-4b-it-GGUF");
    for snap in std::fs::read_dir(format!("{base}/snapshots"))
        .ok()?
        .flatten()
    {
        for f in std::fs::read_dir(snap.path()).ok()?.flatten() {
            let p = f.path();
            if p.extension().is_some_and(|e| e == "gguf") {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }
    None
}

#[tokio::test]
#[ignore = "requires a local gemma-3-4b model"]
async fn run_eval_tool_bootstrap_then_pass() {
    let Some(path) = find_model() else {
        eprintln!("skip: no model");
        return;
    };
    let dir = std::env::temp_dir().join(format!("arkavo-eval-rt-{}", std::process::id()));
    let resolve: ModelResolver = {
        let path = path.clone();
        Arc::new(move |_m: &str| Some(path.clone()))
    };
    let state = Arc::new(EvalState {
        embedder: Arc::new(arkavo_eval::embedder::CharEmbedder),
        baselines: Arc::new(arkavo_eval::baseline_file::FileBaselineStore::new(&dir)),
        prompts: EvalState::default_prompts(),
        resolve_model: resolve,
    });
    let tool = RunEvalTool::new(state);

    let boot = tool
        .execute(json!({ "model": "gemma-3-4b" }))
        .await
        .unwrap();
    assert_eq!(boot["status"], "baseline_bootstrapped");

    let pass = tool
        .execute(json!({ "model": "gemma-3-4b" }))
        .await
        .unwrap();
    assert_eq!(pass["status"], "passed");
    // Temp dir is OS-managed; no explicit cleanup needed in the async context.
}
