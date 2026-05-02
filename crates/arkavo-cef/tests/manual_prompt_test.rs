/// Test to debug manual prompt entry vs --prompt
use arkavo_cef::{ReceivedMessage, UdsTransport};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixListener;

/// Simulates what happens when user manually enters a prompt:
/// 1. Rust is in event loop, polling every 100ms
/// 2. User types and clicks submit
/// 3. C++ sends event
/// 4. Rust should receive it on next poll
#[tokio::test]
async fn test_manual_prompt_entry() {
    let socket_path = format!("/tmp/arkavo_test_manual_{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    // Spawn server that simulates C++ sending events when user types
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        eprintln!("[TEST SERVER] Client connected");

        // Wait a bit (simulating initial page load)
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Now simulate user typing "hello" and clicking submit
        eprintln!("[TEST SERVER] User clicks submit (sending event)");
        let event_data = create_event_message("submit", "#prompt-input", "hello");
        stream.write_all(&event_data).await.unwrap();
        eprintln!("[TEST SERVER] Event sent");

        // Keep connection alive
        tokio::time::sleep(Duration::from_secs(2)).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect client
    let mut transport = UdsTransport::connect(&socket_path).await.unwrap();
    eprintln!("[TEST CLIENT] Connected, starting event loop...");

    // Simulate event loop polling every 100ms
    let mut event_received = false;
    for poll_num in 1..=20 {
        let poll_start = std::time::Instant::now();
        eprintln!(
            "[TEST CLIENT] Poll #{} at {}ms",
            poll_num,
            poll_start.elapsed().as_millis()
        );

        // Try to receive message (like ui.rs does)
        match transport.try_recv_message().await {
            Ok(Some(ReceivedMessage::Event(event))) => {
                eprintln!(
                    "[TEST CLIENT] ✓ Got event on poll #{}: type={}, value={}",
                    poll_num, event.event_type, event.value
                );
                assert_eq!(event.event_type, "submit");
                assert_eq!(event.value, "hello");
                event_received = true;
                break;
            }
            Ok(Some(other)) => {
                eprintln!("[TEST CLIENT] Got other message: {other:?}");
            }
            Ok(None) => {
                eprintln!("[TEST CLIENT] No message on poll #{poll_num}");
            }
            Err(e) => {
                eprintln!("[TEST CLIENT] Error on poll #{poll_num}: {e}");
                break;
            }
        }

        // Wait 100ms before next poll
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(
        event_received,
        "Should have received submit event from manual entry"
    );

    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}

/// Test the scenario where events arrive WHILE Rust is not polling
/// (e.g., during the 2-second initial sleep)
#[tokio::test]
async fn test_events_arrive_before_polling_starts() {
    let socket_path = format!("/tmp/arkavo_test_early_event_{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        eprintln!("[TEST SERVER EARLY] Client connected");

        // Send event IMMEDIATELY (before client starts polling)
        let event_data = create_event_message("submit", "#prompt-input", "early bird");
        stream.write_all(&event_data).await.unwrap();
        eprintln!("[TEST SERVER EARLY] Event sent immediately");

        tokio::time::sleep(Duration::from_secs(2)).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut transport = UdsTransport::connect(&socket_path).await.unwrap();
    eprintln!("[TEST CLIENT EARLY] Connected");

    // Simulate the 2-second sleep from ui.rs (line 89)
    eprintln!("[TEST CLIENT EARLY] Sleeping 2 seconds (like ui.rs does)...");
    tokio::time::sleep(Duration::from_secs(2)).await;
    eprintln!("[TEST CLIENT EARLY] Starting event loop now");

    // Now start polling - the event should still be available
    let mut event_received = false;
    for poll_num in 1..=5 {
        eprintln!("[TEST CLIENT EARLY] Poll #{poll_num}");

        match transport.try_recv_message().await {
            Ok(Some(ReceivedMessage::Event(event))) => {
                eprintln!(
                    "[TEST CLIENT EARLY] ✓ Got event: type={}, value={}",
                    event.event_type, event.value
                );
                assert_eq!(event.event_type, "submit");
                assert_eq!(event.value, "early bird");
                event_received = true;
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                eprintln!("[TEST CLIENT EARLY] No message on poll #{poll_num}");
            }
            Err(e) => {
                eprintln!("[TEST CLIENT EARLY] Error: {e}");
                break;
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(
        event_received,
        "Event sent before polling should still be received when polling starts"
    );

    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}

/// Test multiple rapid events (simulating user clicking submit multiple times)
#[tokio::test]
async fn test_rapid_manual_submits() {
    let socket_path = format!("/tmp/arkavo_test_rapid_{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        // Wait a bit
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Send 3 events rapidly (user clicking submit button multiple times)
        for i in 1..=3 {
            eprintln!("[TEST SERVER RAPID] Sending event #{i}");
            let event_data =
                create_event_message("submit", "#prompt-input", &format!("message {i}"));
            stream.write_all(&event_data).await.unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        eprintln!("[TEST SERVER RAPID] All 3 events sent");

        tokio::time::sleep(Duration::from_secs(2)).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut transport = UdsTransport::connect(&socket_path).await.unwrap();
    eprintln!("[TEST CLIENT RAPID] Connected, starting polling");

    let mut events_received = Vec::new();

    // Poll for up to 2 seconds
    for poll_num in 1..=20 {
        match transport.try_recv_message().await {
            Ok(Some(ReceivedMessage::Event(event))) => {
                eprintln!(
                    "[TEST CLIENT RAPID] Got event on poll #{}: {}",
                    poll_num, event.value
                );
                events_received.push(event.value.clone());
            }
            Ok(Some(_)) => {}
            Ok(None) => {}
            Err(e) => {
                eprintln!("[TEST CLIENT RAPID] Error: {e}");
                break;
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Should receive all 3 events (but UI should probably debounce/dedupe them)
    assert!(
        !events_received.is_empty(),
        "Should receive at least 1 event, got: {events_received:?}"
    );
    eprintln!(
        "[TEST CLIENT RAPID] Received {} events: {:?}",
        events_received.len(),
        events_received
    );

    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}

fn create_event_message(event_type: &str, selector: &str, value: &str) -> Vec<u8> {
    let mut buffer = Vec::new();

    // Message type 0x02 for event
    buffer.push(0x02);

    let write_string = |buf: &mut Vec<u8>, s: &str| {
        let bytes = s.as_bytes();
        buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(bytes);
    };

    write_string(&mut buffer, event_type);
    write_string(&mut buffer, selector);
    write_string(&mut buffer, "target_id");
    write_string(&mut buffer, value);
    write_string(&mut buffer, "{}");

    // Frame it
    let mut framed = Vec::new();
    framed.extend_from_slice(&(buffer.len() as u32).to_le_bytes());
    framed.extend_from_slice(&buffer);
    framed
}
