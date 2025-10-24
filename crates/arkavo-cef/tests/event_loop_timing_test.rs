use arkavo_cef::{ReceivedMessage, UdsTransport};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixListener;

/// Test realistic event loop timing scenario
///
/// This simulates the actual production scenario:
/// - Event loop polls every 100ms (ui.rs:104)
/// - try_recv_message() has 10ms timeout (dom_commands.rs:63)
/// - User clicks Submit button at arbitrary time
///
/// The issue: Events arriving between polls are missed because the 10ms timeout
/// is too short relative to the 100ms polling interval.
#[tokio::test]
async fn test_event_loop_with_realistic_timing() {
    let socket_path = format!("/tmp/arkavo_test_timing_{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    // Spawn server that simulates user clicking Submit 150ms after start
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        eprintln!("[TEST SERVER] Client connected");

        // Wait 150ms to simulate user thinking and then clicking Submit
        tokio::time::sleep(Duration::from_millis(150)).await;

        eprintln!("[TEST SERVER] User clicks Submit (150ms after start)");
        let event_data = create_event_message("submit", "#input", "test input");
        stream.write_all(&event_data).await.unwrap();

        eprintln!("[TEST SERVER] Event sent");

        // Keep connection alive
        tokio::time::sleep(Duration::from_millis(500)).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect client
    let mut transport = UdsTransport::connect(&socket_path).await.unwrap();
    eprintln!("[TEST CLIENT] Connected");

    // Simulate event loop polling every 100ms
    let start = tokio::time::Instant::now();
    let mut event_received = false;
    let mut poll_count = 0;

    // Poll for up to 1 second (10 polls at 100ms each)
    while start.elapsed() < Duration::from_secs(1) {
        poll_count += 1;
        let elapsed = start.elapsed().as_millis();
        eprintln!("[TEST CLIENT] Poll #{} at {}ms", poll_count, elapsed);

        // Try to receive message (this uses 10ms timeout in dom_commands.rs:63)
        match transport.try_recv_message().await {
            Ok(Some(ReceivedMessage::Event(event))) => {
                eprintln!(
                    "[TEST CLIENT] ✓ Event received at {}ms: type={}, value={}",
                    elapsed, event.event_type, event.value
                );
                assert_eq!(event.event_type, "submit");
                assert_eq!(event.value, "test input");
                event_received = true;
                break;
            }
            Ok(Some(other)) => {
                eprintln!("[TEST CLIENT] Got other message: {:?}", other);
            }
            Ok(None) => {
                eprintln!("[TEST CLIENT] No message (timeout or no data)");
            }
            Err(e) => {
                eprintln!("[TEST CLIENT] Error: {}", e);
            }
        }

        // Wait 100ms before next poll (simulates ui.rs:104)
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(
        event_received,
        "Event should have been received within 10 polls. \
         Issue: 10ms timeout in try_recv_message is too short for 100ms polling interval. \
         Event sent at ~150ms should be caught by poll #2 (~200ms) or poll #3 (~300ms)."
    );

    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}

/// Test that increasing timeout to 50ms helps catch events
///
/// This test demonstrates that a longer timeout would catch events more reliably.
/// Currently, we can't easily test this without modifying the code, but this test
/// documents the expected behavior.
#[tokio::test]
async fn test_event_loop_with_longer_timeout() {
    let socket_path = format!("/tmp/arkavo_test_timing_long_{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    // Spawn server that sends event at 150ms
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        eprintln!("[TEST SERVER LONG] Client connected");

        tokio::time::sleep(Duration::from_millis(150)).await;

        eprintln!("[TEST SERVER LONG] Sending event at 150ms");
        let event_data = create_event_message("submit", "#input", "test with longer timeout");
        stream.write_all(&event_data).await.unwrap();

        tokio::time::sleep(Duration::from_millis(500)).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect client
    let mut transport = UdsTransport::connect(&socket_path).await.unwrap();
    eprintln!("[TEST CLIENT LONG] Connected");

    let start = tokio::time::Instant::now();
    let mut event_received = false;
    let mut poll_count = 0;

    // Poll with manual 50ms timeout (simulating proposed fix)
    while start.elapsed() < Duration::from_secs(1) {
        poll_count += 1;
        let elapsed = start.elapsed().as_millis();
        eprintln!("[TEST CLIENT LONG] Poll #{} at {}ms", poll_count, elapsed);

        // Use custom 50ms timeout (proposed fix)
        match tokio::time::timeout(Duration::from_millis(50), transport.try_recv_message()).await {
            Ok(Ok(Some(ReceivedMessage::Event(event)))) => {
                eprintln!(
                    "[TEST CLIENT LONG] ✓ Event received at {}ms with 50ms timeout: type={}, value={}",
                    elapsed, event.event_type, event.value
                );
                assert_eq!(event.event_type, "submit");
                assert_eq!(event.value, "test with longer timeout");
                event_received = true;
                break;
            }
            Ok(Ok(Some(_))) => {}
            Ok(Ok(None)) => {
                eprintln!("[TEST CLIENT LONG] No message");
            }
            Ok(Err(e)) => {
                eprintln!("[TEST CLIENT LONG] Error: {}", e);
            }
            Err(_) => {
                eprintln!("[TEST CLIENT LONG] 50ms timeout");
            }
        }

        // Wait 100ms before next poll
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // With 50ms timeout, we should catch the event more reliably
    // This test shows what SHOULD happen after fixing the timeout
    assert!(
        event_received,
        "Event should be received with 50ms timeout. \
         Event sent at ~150ms should be caught by poll #2 (starts at ~200ms, waits 50ms = catches event at ~250ms max)"
    );

    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}

/// Test the queue draining pattern from ui.rs:151-176
///
/// This verifies that the loop pattern that drains all messages works correctly
/// when messages arrive during the draining process.
#[tokio::test]
async fn test_message_queue_draining_pattern() {
    let socket_path = format!("/tmp/arkavo_test_drain_{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    // Spawn server that sends multiple events in quick succession
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        eprintln!("[TEST SERVER DRAIN] Client connected");

        // Wait a bit, then send 3 events rapidly
        tokio::time::sleep(Duration::from_millis(100)).await;

        for i in 1..=3 {
            eprintln!("[TEST SERVER DRAIN] Sending event #{}", i);
            let event_data = create_event_message("submit", "#input", &format!("event {}", i));
            stream.write_all(&event_data).await.unwrap();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        eprintln!("[TEST SERVER DRAIN] All events sent");
        tokio::time::sleep(Duration::from_millis(500)).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect client
    let mut transport = UdsTransport::connect(&socket_path).await.unwrap();
    eprintln!("[TEST CLIENT DRAIN] Connected");

    // Wait for events to arrive
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Drain all messages (pattern from ui.rs:151-176)
    let mut events_received = Vec::new();
    loop {
        match transport.try_recv_message().await {
            Ok(Some(ReceivedMessage::Event(event))) => {
                eprintln!("[TEST CLIENT DRAIN] Got event: {}", event.value);
                events_received.push(event.value.clone());
            }
            Ok(Some(_)) => {
                eprintln!("[TEST CLIENT DRAIN] Got non-event message");
            }
            Ok(None) => {
                eprintln!("[TEST CLIENT DRAIN] No more messages, breaking");
                break;
            }
            Err(e) => {
                eprintln!("[TEST CLIENT DRAIN] Error: {}", e);
                break;
            }
        }
    }

    // Should have received all 3 events
    assert_eq!(
        events_received.len(),
        3,
        "Should receive all 3 events. Got: {:?}",
        events_received
    );
    assert_eq!(events_received[0], "event 1");
    assert_eq!(events_received[1], "event 2");
    assert_eq!(events_received[2], "event 3");

    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}

fn create_event_message(event_type: &str, selector: &str, value: &str) -> Vec<u8> {
    let mut buffer = Vec::new();

    // Message type (0x02 = event)
    buffer.push(0x02);

    // Helper to write string
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
