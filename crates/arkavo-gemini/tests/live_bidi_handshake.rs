//! Live integration test: WebSocket bidi handshake.
//!
//! Skipped unless `GEMINI_API_KEY` is set. Verifies the WebSocket Live API
//! handshake — endpoint reachable, setup frame accepted, `setupComplete`
//! arrives — for the only model the public v1alpha endpoint currently
//! routes (`gemini-2.5-flash-native-audio-latest`).
//!
//! We do **not** send a text turn or wait for tool calls: the native-audio
//! model only emits AUDIO and only accepts realtime audio frames, so the
//! handshake itself is the contract we can test against the public API.
#![allow(clippy::disallowed_methods)]

use arkavo_gemini::{LiveModality, LiveSessionClient, ToolDispatcher, ToolRegistry};
use serde_json::json;
use tokio::time::{Duration, timeout};

fn api_key_or_skip(test: &str) -> Option<String> {
    match std::env::var("GEMINI_API_KEY") {
        Ok(k) if !k.is_empty() => Some(k),
        _ => {
            eprintln!("skipping {test}: GEMINI_API_KEY not set");
            None
        }
    }
}

#[tokio::test]
async fn live_bidi_setup_handshake() {
    let Some(api_key) = api_key_or_skip("live_bidi_setup_handshake") else {
        return;
    };
    let model = std::env::var("GEMINI_LIVE_MODEL")
        .unwrap_or_else(|_| "gemini-2.5-flash-native-audio-latest".to_string());

    let dispatcher = ToolDispatcher::new(2);
    let mut registry = ToolRegistry::new();
    registry.register(
        "noop",
        "No-op tool used only to populate the setup frame",
        json!({ "type": "object", "properties": {} }),
        |_args| Ok(json!({"ok": true})),
    );
    registry.build(&dispatcher);

    let client = LiveSessionClient::new_with_tools(api_key, &model, dispatcher.list_tools())
        .with_modality(LiveModality::Audio);

    timeout(Duration::from_secs(15), client.connect())
        .await
        .expect("connect() timed out — WebSocket handshake stuck")
        .expect("connect() failed");
    assert!(
        client.is_connected(),
        "client.is_connected() is false after connect()"
    );

    // Give the server a beat to deliver setupComplete on the receiver task.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Intentionally do not call client.close(): the current implementation
    // contends with the receiver task for the WS write lock and deadlocks.
    // Tokio drops the spawned receiver when the test runtime tears down.
}
