/// Regression test for the double-async timeout bug
///
/// Bug: DOMCommandBuilder::try_recv_message was wrapping the already-non-blocking
/// transport.try_recv_message() with a 10ms timeout, causing it to timeout even
/// when messages were available.
///
/// Scenario:
/// 1. User clicks Submit button in UI
/// 2. C++ sends event via UDS (takes ~1-2ms)
/// 3. transport.try_recv_message() is non-blocking but might take 1-2ms
/// 4. tokio::timeout(10ms) wraps it, but if ANY delay happens, returns Err(timeout)
/// 5. The timeout error is converted to Ok(None), silently dropping the event
///
/// This test verifies that try_recv_message doesn't timeout when receiving
/// messages that arrive with realistic timing delays.
use arkavo_cef::{ReceivedMessage, UdsTransport};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixListener;

#[tokio::test]
async fn test_no_timeout_on_fast_messages() {
    let socket_path = format!("/tmp/arkavo_test_timeout_bug_{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    // Server that sends event with realistic timing
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        // Simulate realistic delay (network latency, scheduling)
        tokio::time::sleep(Duration::from_millis(5)).await;

        // Send event
        let event_data = create_event_message("submit", "#prompt-input", "test");
        stream.write_all(&event_data).await.unwrap();
        eprintln!("[TEST SERVER] Event sent after 5ms delay");

        tokio::time::sleep(Duration::from_millis(100)).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut transport = UdsTransport::connect(&socket_path).await.unwrap();
    eprintln!("[TEST CLIENT] Connected, starting poll loop");

    // Try to receive the message with realistic polling (like ui.rs does)
    let mut received = false;
    for attempt in 1..=10 {
        eprintln!("[TEST CLIENT] Poll attempt {}", attempt);

        match transport.try_recv_message().await {
            Ok(Some(ReceivedMessage::Event(event))) => {
                eprintln!("[TEST CLIENT] ✓ Received event: {}", event.value);
                assert_eq!(event.event_type, "submit");
                assert_eq!(event.value, "test");
                received = true;
                break;
            }
            Ok(Some(other)) => {
                eprintln!("[TEST CLIENT] Got other message: {:?}", other);
            }
            Ok(None) => {
                eprintln!("[TEST CLIENT] No message (will retry)");
            }
            Err(e) => {
                eprintln!("[TEST CLIENT] Error: {}", e);
                panic!("Should not get error, got: {}", e);
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(
        received,
        "Should have received the event despite timing delays"
    );
    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}

/// Test that rapid successive calls to try_recv_message don't cause timeouts
#[tokio::test]
async fn test_rapid_polling_no_timeout() {
    let socket_path = format!("/tmp/arkavo_test_rapid_poll_{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        // Send event after a brief delay
        tokio::time::sleep(Duration::from_millis(50)).await;
        let event_data = create_event_message("submit", "#prompt-input", "rapid test");
        stream.write_all(&event_data).await.unwrap();

        tokio::time::sleep(Duration::from_millis(100)).await;
    });

    tokio::time::sleep(Duration::from_millis(10)).await;

    let mut transport = UdsTransport::connect(&socket_path).await.unwrap();

    // Simulate aggressive polling (every 10ms, like some event loops do)
    let mut received = false;
    for attempt in 1..=20 {
        match transport.try_recv_message().await {
            Ok(Some(ReceivedMessage::Event(event))) => {
                eprintln!(
                    "[TEST RAPID] ✓ Received on attempt {}: {}",
                    attempt, event.value
                );
                assert_eq!(event.value, "rapid test");
                received = true;
                break;
            }
            Ok(None) => {
                // This is expected when no message available
            }
            Err(e) => {
                panic!("Rapid polling should never error, got: {}", e);
            }
            _ => {}
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert!(received, "Rapid polling should not miss messages");
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
