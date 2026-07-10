//! Mock-server integration tests for the Gemini Live (WebSocket) client.
//!
//! These tests spin up a local WebSocket server so they exercise the real
//! `LiveSessionClient` message framing without hitting the Gemini API.
#![allow(clippy::disallowed_methods)]

use arkavo_gemini::{LiveModality, LiveSessionClient, ToolDispatcher, ToolRegistry};
use arkavo_test_macros::spec;
use base64::{Engine as _, engine::general_purpose};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{WebSocketStream, accept_async};

type MockWs = WebSocketStream<tokio::net::TcpStream>;

async fn start_mock_ws<F, Fut>(handler: F) -> (u16, tokio::task::JoinHandle<()>)
where
    F: FnOnce(MockWs) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws = accept_async(stream).await.unwrap();
        handler(ws).await;
    });
    (port, handle)
}

fn b64(data: &[u8]) -> String {
    general_purpose::STANDARD.encode(data)
}

#[spec("GEM-005")]
#[tokio::test]
async fn live_send_audio_content() {
    let (port, server) = start_mock_ws(|mut ws| async move {
        let setup = ws.next().await.unwrap().unwrap();
        let setup_text = setup.to_text().unwrap();
        let setup_value: Value = serde_json::from_str(setup_text).unwrap();
        assert_eq!(
            setup_value["setup"]["generationConfig"]["responseModalities"][0],
            "AUDIO"
        );

        ws.send(Message::Text(r#"{"setupComplete":{}}"#.to_string()))
            .await
            .unwrap();

        let audio_msg = ws.next().await.unwrap().unwrap();
        let audio_text = audio_msg.to_text().unwrap();
        let audio_value: Value = serde_json::from_str(audio_text).unwrap();
        let parts = audio_value["clientContent"]["turns"][0]["parts"]
            .as_array()
            .unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["inlineData"]["mimeType"], "audio/pcm");
        assert_eq!(parts[0]["inlineData"]["data"], b64(b"\x00\x01\x02"));

        ws.send(Message::Close(None)).await.unwrap();
    })
    .await;

    let client = LiveSessionClient::new_with_tools(
        "test_api_key",
        "gemini-2.5-flash-native-audio-latest",
        vec![],
    )
    .with_modality(LiveModality::Audio)
    .with_endpoint_url(format!("ws://127.0.0.1:{port}/ws"));

    timeout(Duration::from_secs(5), client.connect())
        .await
        .expect("connect timed out")
        .expect("connect failed");
    assert!(client.is_connected());

    client
        .send_audio(b64(b"\x00\x01\x02"), "audio/pcm".to_string())
        .await
        .expect("send_audio failed");

    timeout(Duration::from_secs(5), client.close())
        .await
        .expect("close timed out")
        .expect("close failed");
    assert!(!client.is_connected());

    server.await.unwrap();
}

#[spec("GEM-006")]
#[tokio::test]
async fn live_receives_server_tool_call() {
    let (port, server) = start_mock_ws(|mut ws| async move {
        let _setup = ws.next().await.unwrap().unwrap();
        ws.send(Message::Text(r#"{"setupComplete":{}}"#.to_string()))
            .await
            .unwrap();

        ws.send(Message::Text(
            r#"{"toolCall":{"functionCalls":[{"name":"echo","args":{"message":"hello"},"id":"call-1"}]}}"#
                .to_string(),
        ))
        .await
        .unwrap();

        let response = ws.next().await.unwrap().unwrap();
        let response_text = response.to_text().unwrap();
        let response_value: Value = serde_json::from_str(response_text).unwrap();
        assert_eq!(
            response_value["toolResponse"]["functionResponses"][0]["id"],
            "call-1"
        );
        assert_eq!(
            response_value["toolResponse"]["functionResponses"][0]["response"]["echo"],
            "hello"
        );

        ws.send(Message::Close(None)).await.unwrap();
    })
    .await;

    let dispatcher = ToolDispatcher::new(2);
    let mut registry = ToolRegistry::new();
    registry.register(
        "echo",
        "Echoes the input",
        json!({"type": "object"}),
        |args| Ok(json!({"echo": args["message"]})),
    );
    registry.build(&dispatcher);

    let client = LiveSessionClient::new_with_tools(
        "test_api_key",
        "gemini-2.5-flash-native-audio-latest",
        dispatcher.list_tools(),
    )
    .with_endpoint_url(format!("ws://127.0.0.1:{port}/ws"));

    timeout(Duration::from_secs(5), client.connect())
        .await
        .expect("connect timed out")
        .expect("connect failed");

    let calls = timeout(Duration::from_secs(5), client.receive_tool_calls())
        .await
        .expect("receive_tool_calls timed out")
        .expect("receive_tool_calls failed");

    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "echo");
    assert_eq!(calls[0].args, json!({"message": "hello"}));
    assert_eq!(calls[0].id, "call-1");

    let results = dispatcher.dispatch(calls).await;
    assert_eq!(results.len(), 1);
    let (call_id, result) = results.into_iter().next().unwrap();
    let response = result.expect("tool execution failed");

    client
        .send_tool_response(call_id, response)
        .await
        .expect("send_tool_response failed");

    timeout(Duration::from_secs(5), client.close())
        .await
        .expect("close timed out")
        .expect("close failed");

    server.await.unwrap();
}

#[spec("GEM-006")]
#[tokio::test]
async fn live_ignores_empty_tool_call() {
    let (port, server) = start_mock_ws(|mut ws| async move {
        let _setup = ws.next().await.unwrap().unwrap();
        ws.send(Message::Text(r#"{"setupComplete":{}}"#.to_string()))
            .await
            .unwrap();

        ws.send(Message::Text(
            r#"{"toolCall":{"functionCalls":[]}}"#.to_string(),
        ))
        .await
        .unwrap();

        ws.send(Message::Close(None)).await.unwrap();
    })
    .await;

    let client = LiveSessionClient::new_with_tools(
        "test_api_key",
        "gemini-2.5-flash-native-audio-latest",
        vec![],
    )
    .with_endpoint_url(format!("ws://127.0.0.1:{port}/ws"));

    timeout(Duration::from_secs(5), client.connect())
        .await
        .expect("connect timed out")
        .expect("connect failed");

    let result = timeout(Duration::from_millis(500), client.receive_tool_calls()).await;
    assert!(
        result.is_err() || result.unwrap().is_err(),
        "empty toolCall should not produce a receive_tool_calls result"
    );

    timeout(Duration::from_secs(5), client.close())
        .await
        .expect("close timed out")
        .expect("close failed");

    server.await.unwrap();
}

#[spec("GEM-004")]
#[tokio::test]
async fn live_handshake_honors_setup_config() {
    let (port, server) = start_mock_ws(|mut ws| async move {
        let setup = ws.next().await.unwrap().unwrap();
        let setup_text = setup.to_text().unwrap();
        let setup_value: Value = serde_json::from_str(setup_text).unwrap();

        assert!(
            setup_value.get("setup").is_some(),
            "first client message must be the setup frame"
        );
        assert_eq!(
            setup_value["setup"]["model"],
            "models/gemini-2.5-flash-native-audio-latest"
        );
        assert_eq!(
            setup_value["setup"]["generationConfig"]["responseModalities"][0],
            "TEXT"
        );
        assert!(
            setup_value["setup"].get("tools").is_none(),
            "setup frame must omit tools when none are registered"
        );

        ws.send(Message::Text(r#"{"setupComplete":{}}"#.to_string()))
            .await
            .unwrap();

        // Drain until the client closes the socket.
        while let Some(Ok(msg)) = ws.next().await {
            if msg.is_close() {
                break;
            }
        }
    })
    .await;

    let client = LiveSessionClient::new_with_tools(
        "test_api_key",
        "gemini-2.5-flash-native-audio-latest",
        vec![],
    )
    .with_modality(LiveModality::Text)
    .with_endpoint_url(format!("ws://127.0.0.1:{port}/ws"));

    timeout(Duration::from_secs(5), client.connect())
        .await
        .expect("connect timed out")
        .expect("connect failed");
    assert!(
        client.is_connected(),
        "client must be connected after successful handshake"
    );

    timeout(Duration::from_secs(5), client.close())
        .await
        .expect("close timed out")
        .expect("close failed");
    assert!(
        !client.is_connected(),
        "client must be disconnected after close"
    );

    server.await.unwrap();
}
