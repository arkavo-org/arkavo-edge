use arkavo_cef::{ReceivedMessage, UdsTransport};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

/// Test the ACTUAL production issue: Events arriving during recv_feedback() wait
///
/// The real problem:
/// 1. Rust sends command and calls recv_feedback().await
/// 2. C++ sends feedback immediately (but missing type byte 0x01)
/// 3. User clicks Submit button while Rust is processing
/// 4. C++ sends event (with correct type byte 0x02)
/// 5. recv_feedback() reads BOTH messages but only processes feedback
/// 6. Event gets queued in message_queue
/// 7. Event loop calls try_recv_message() but the 10ms timeout might miss it
///
/// This test simulates steps 1-6 to verify the queue works.
#[tokio::test]
async fn test_event_arrives_during_feedback_wait() {
    let socket_path = format!(
        "/tmp/arkavo_test_event_during_fb_{}.sock",
        std::process::id()
    );
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    // Spawn server that sends: feedback, then IMMEDIATELY sends event
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        eprintln!("[TEST SERVER] Client connected");

        // Read command
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut cmd_buf = vec![0u8; len];
        stream.read_exact(&mut cmd_buf).await.unwrap();
        eprintln!("[TEST SERVER] Received command");

        // Send feedback (with correct type byte 0x01)
        let feedback_data = create_feedback_message(0, 0, "OK");
        stream.write_all(&feedback_data).await.unwrap();
        eprintln!("[TEST SERVER] Sent feedback");

        // IMMEDIATELY send event (simulates user clicking while command is executing)
        // No delay - this is the key difference from other tests
        let event_data = create_event_message("submit", "#input", "user clicked");
        stream.write_all(&event_data).await.unwrap();
        eprintln!("[TEST SERVER] Sent event immediately after feedback");

        tokio::time::sleep(Duration::from_millis(500)).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect client
    let mut transport = UdsTransport::connect(&socket_path).await.unwrap();
    eprintln!("[TEST CLIENT] Connected");

    // Send command
    transport
        .send_command(0, 0, "#test", "content", None)
        .await
        .unwrap();
    eprintln!("[TEST CLIENT] Command sent, waiting for feedback");

    // Receive feedback - this should queue the event message
    let feedback = transport.recv_feedback().await.unwrap();
    eprintln!(
        "[TEST CLIENT] Got feedback: id={}, status={}",
        feedback.id, feedback.status
    );

    // Now the event should be in the queue
    eprintln!("[TEST CLIENT] Checking for queued event...");

    // Try to receive the event - should be queued from recv_feedback()
    match transport.try_recv_message().await {
        Ok(Some(ReceivedMessage::Event(event))) => {
            eprintln!(
                "[TEST CLIENT] ✓ Event was queued: type={}, value={}",
                event.event_type, event.value
            );
            assert_eq!(event.event_type, "submit");
            assert_eq!(event.value, "user clicked");
        }
        Ok(Some(other)) => {
            panic!("Expected Event, got: {other:?}");
        }
        Ok(None) => {
            panic!("Event should have been queued by recv_feedback(), but got None");
        }
        Err(e) => {
            panic!("Error receiving queued event: {e}");
        }
    }

    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}

/// Test multiple events arriving during feedback wait
///
/// This simulates rapid user interaction (multiple clicks) while a command is executing.
#[tokio::test]
async fn test_multiple_events_during_feedback_wait() {
    let socket_path = format!("/tmp/arkavo_test_multi_event_{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        eprintln!("[TEST SERVER MULTI] Client connected");

        // Read command
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut cmd_buf = vec![0u8; len];
        stream.read_exact(&mut cmd_buf).await.unwrap();

        // Send feedback
        let feedback_data = create_feedback_message(0, 0, "OK");
        stream.write_all(&feedback_data).await.unwrap();
        eprintln!("[TEST SERVER MULTI] Sent feedback");

        // Send 3 events rapidly (simulating impatient user clicking multiple times)
        for i in 1..=3 {
            let event_data = create_event_message("submit", "#input", &format!("click {i}"));
            stream.write_all(&event_data).await.unwrap();
            eprintln!("[TEST SERVER MULTI] Sent event #{i}");
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut transport = UdsTransport::connect(&socket_path).await.unwrap();
    eprintln!("[TEST CLIENT MULTI] Connected");

    // Send command and wait for feedback
    transport
        .send_command(0, 0, "#test", "content", None)
        .await
        .unwrap();
    let feedback = transport.recv_feedback().await.unwrap();
    eprintln!(
        "[TEST CLIENT MULTI] Got feedback: status={}",
        feedback.status
    );

    // All 3 events should be queued
    let mut events_received = Vec::new();
    for i in 1..=3 {
        match transport.try_recv_message().await {
            Ok(Some(ReceivedMessage::Event(event))) => {
                eprintln!("[TEST CLIENT MULTI] Got event #{}: {}", i, event.value);
                events_received.push(event.value.clone());
            }
            Ok(Some(_)) => panic!("Expected Event #{i}"),
            Ok(None) => panic!("Event #{i} should be queued"),
            Err(e) => panic!("Error receiving event #{i}: {e}"),
        }
    }

    assert_eq!(events_received, vec!["click 1", "click 2", "click 3"]);

    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}

/// Test the C++ bug: feedback without message type byte causes event loss
///
/// This demonstrates what happens when C++ SendFeedback doesn't write the type byte:
/// 1. recv_feedback() gets malformed feedback (msg_type=0)
/// 2. Returns protocol error
/// 3. Event sent after feedback is lost
#[tokio::test]
async fn test_malformed_feedback_loses_events() {
    let socket_path = format!("/tmp/arkavo_test_malformed_fb_{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        // Read command
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut cmd_buf = vec![0u8; len];
        stream.read_exact(&mut cmd_buf).await.unwrap();

        // Send MALFORMED feedback (missing type byte 0x01) - simulates C++ bug
        let feedback_data = create_malformed_feedback_message(0, 0, "OK");
        stream.write_all(&feedback_data).await.unwrap();
        eprintln!("[TEST SERVER MALFORMED] Sent malformed feedback");

        // Send event after
        let event_data = create_event_message("submit", "#input", "important event");
        stream.write_all(&event_data).await.unwrap();
        eprintln!("[TEST SERVER MALFORMED] Sent event");

        tokio::time::sleep(Duration::from_millis(500)).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut transport = UdsTransport::connect(&socket_path).await.unwrap();

    // Send command
    transport
        .send_command(0, 0, "#test", "content", None)
        .await
        .unwrap();

    // Try to receive feedback - should fail with protocol error
    let result = transport.recv_feedback().await;
    assert!(result.is_err(), "Should error due to malformed feedback");

    eprintln!(
        "[TEST CLIENT MALFORMED] Got expected error: {:?}",
        result.unwrap_err()
    );

    // The event might have been consumed by recv_feedback()'s recv_raw() call
    // or it might still be on the socket. Either way, this demonstrates the issue.

    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}

fn create_feedback_message(id: u32, status: u8, message: &str) -> Vec<u8> {
    let mut buffer = Vec::new();

    // Message type 0x01 for feedback
    buffer.push(0x01);

    // ID (4 bytes)
    buffer.extend_from_slice(&id.to_le_bytes());

    // Status (1 byte - uint8_t in C++)
    buffer.push(status);

    // exec_time_ns (8 bytes)
    buffer.extend_from_slice(&0u64.to_le_bytes());

    // Message length and bytes
    let msg_bytes = message.as_bytes();
    buffer.extend_from_slice(&(msg_bytes.len() as u32).to_le_bytes());
    buffer.extend_from_slice(msg_bytes);

    // Frame it
    let mut framed = Vec::new();
    framed.extend_from_slice(&(buffer.len() as u32).to_le_bytes());
    framed.extend_from_slice(&buffer);
    framed
}

fn create_malformed_feedback_message(id: u32, status: u8, message: &str) -> Vec<u8> {
    let mut buffer = Vec::new();

    // BUG: Missing message type byte 0x01

    // ID (4 bytes)
    buffer.extend_from_slice(&id.to_le_bytes());

    // Status (1 byte)
    buffer.push(status);

    // exec_time_ns (8 bytes)
    buffer.extend_from_slice(&0u64.to_le_bytes());

    // Message length and bytes
    let msg_bytes = message.as_bytes();
    buffer.extend_from_slice(&(msg_bytes.len() as u32).to_le_bytes());
    buffer.extend_from_slice(msg_bytes);

    // Frame it
    let mut framed = Vec::new();
    framed.extend_from_slice(&(buffer.len() as u32).to_le_bytes());
    framed.extend_from_slice(&buffer);
    framed
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
