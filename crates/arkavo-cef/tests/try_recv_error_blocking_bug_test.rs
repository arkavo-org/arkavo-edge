#![allow(clippy::disallowed_methods)]

/// Regression test for the try_recv_error() blocking bug
///
/// Bug: DOMCommandBuilder::try_recv_error() was calling the BLOCKING
/// transport.recv_error() wrapped in a timeout. When the UI event loop
/// called try_recv_error() every 100ms, it would block for 10ms each time.
///
/// When a user event arrived, recv_error() would consume it but fail to
/// deserialize it (since it's not an error), losing the event.
///
/// Scenario from ui.rs event loop:
/// 1. Line 117: try_recv_error() called every iteration (polls for console errors)
/// 2. User clicks Submit button
/// 3. C++ sends Event (msg_type=0x02)
/// 4. try_recv_error() is blocking inside recv_error() -> recv_raw()
/// 5. recv_raw() reads the Event but recv_error() expects Error message
/// 6. Event is lost because recv_error() can't deserialize it
/// 7. try_recv_message() never sees the event
///
/// This test verifies that try_recv_error() doesn't consume non-error messages.
use arkavo_cef::{DOMCommandBuilder, ReceivedMessage, UdsTransport};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixListener;

#[tokio::test]
async fn test_try_recv_error_does_not_consume_events() {
    let socket_path = format!(
        "/tmp/arkavo_test_recv_error_bug_{}.sock",
        std::process::id()
    );
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    // Server simulates UI scenario:
    // 1. Application is running (no errors)
    // 2. User clicks Submit
    // 3. Event is sent
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        // Wait a bit (simulate initial page load)
        tokio::time::sleep(Duration::from_millis(100)).await;

        // User clicks Submit - C++ sends event
        eprintln!("[TEST SERVER] User clicked Submit, sending event");
        let event_data = create_event_message("submit", "#prompt-input", "hello world");
        stream.write_all(&event_data).await.unwrap();

        tokio::time::sleep(Duration::from_millis(500)).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let transport = UdsTransport::connect(&socket_path).await.unwrap();
    let mut commands = DOMCommandBuilder::new(transport);

    eprintln!("[TEST CLIENT] Simulating ui.rs event loop...");

    // Simulate the event loop from ui.rs:
    // Every 100ms, poll for errors (line 117)
    let mut event_received = false;
    for iteration in 1..=10 {
        eprintln!("[TEST CLIENT] Event loop iteration {iteration}");

        // Line 117 in ui.rs: Poll for console errors
        match commands.try_recv_error().await {
            Ok(Some(error)) => {
                eprintln!("[TEST CLIENT] Got error: {:?}", error.error_type);
            }
            Ok(None) => {
                // No error (expected)
            }
            Err(e) => {
                panic!("try_recv_error should not error, got: {e}");
            }
        }

        // Line 152 in ui.rs: Poll for all messages (events, feedback, errors)
        loop {
            match commands.try_recv_message().await {
                Ok(Some(ReceivedMessage::Event(event))) => {
                    eprintln!(
                        "[TEST CLIENT] ✓ Got event on iteration {}: type={}, value={}",
                        iteration, event.event_type, event.value
                    );
                    assert_eq!(event.event_type, "submit");
                    assert_eq!(event.value, "hello world");
                    event_received = true;
                    break;
                }
                Ok(Some(other)) => {
                    eprintln!("[TEST CLIENT] Got other message: {other:?}");
                }
                Ok(None) => {
                    // No more messages
                    break;
                }
                Err(e) => {
                    panic!("try_recv_message should not error, got: {e}");
                }
            }
        }

        if event_received {
            break;
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(
        event_received,
        "Event should be received by try_recv_message(), not consumed by try_recv_error()"
    );

    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}

/// Test that multiple try_recv_error() calls don't block or lose events
#[tokio::test]
async fn test_rapid_try_recv_error_polling() {
    let socket_path = format!(
        "/tmp/arkavo_test_rapid_error_poll_{}.sock",
        std::process::id()
    );
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        // Send event after multiple try_recv_error polls
        tokio::time::sleep(Duration::from_millis(150)).await;
        let event_data = create_event_message("submit", "#input", "test");
        stream.write_all(&event_data).await.unwrap();

        tokio::time::sleep(Duration::from_millis(200)).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let transport = UdsTransport::connect(&socket_path).await.unwrap();
    let mut commands = DOMCommandBuilder::new(transport);

    // Aggressively poll for errors (simulate rapid event loop)
    let mut poll_count = 0;
    let mut event_received = false;

    for _ in 0..20 {
        // Rapidly call try_recv_error (should not block)
        let start = std::time::Instant::now();
        let _ = commands.try_recv_error().await;
        let elapsed = start.elapsed();

        poll_count += 1;

        // Verify it's actually non-blocking (should complete quickly)
        assert!(
            elapsed < Duration::from_millis(20),
            "try_recv_error should be non-blocking, took {elapsed:?}"
        );

        // Check for the event
        if let Ok(Some(ReceivedMessage::Event(event))) = commands.try_recv_message().await {
            eprintln!(
                "[TEST RAPID] ✓ Got event after {} polls: {}",
                poll_count, event.value
            );
            event_received = true;
            break;
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert!(
        event_received,
        "Should receive event even with aggressive error polling"
    );

    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}

fn create_event_message(event_type: &str, selector: &str, value: &str) -> Vec<u8> {
    let mut buffer = Vec::new();

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

    let mut framed = Vec::new();
    framed.extend_from_slice(&(buffer.len() as u32).to_le_bytes());
    framed.extend_from_slice(&buffer);
    framed
}
