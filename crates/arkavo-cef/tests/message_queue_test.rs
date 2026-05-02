use arkavo_cef::{ReceivedMessage, UdsTransport};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

/// Test that events received during feedback wait are queued properly
#[tokio::test]
async fn test_event_queuing_during_feedback_wait() {
    let socket_path = format!("/tmp/arkavo_test_queue_{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    // Create UDS server
    let listener = UnixListener::bind(&socket_path).unwrap();

    // Spawn server task that sends: feedback, event, feedback
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        // Read first command
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut cmd_buf = vec![0u8; len];
        stream.read_exact(&mut cmd_buf).await.unwrap();

        // Send feedback for command 0
        let feedback_data = create_feedback_message(0, 0, "OK");
        stream.write_all(&feedback_data).await.unwrap();

        // Immediately send an event (simulating user interaction)
        tokio::time::sleep(Duration::from_millis(10)).await;
        let event_data = create_event_message("submit", "#input", "test");
        stream.write_all(&event_data).await.unwrap();

        // Read second command
        stream.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut cmd_buf = vec![0u8; len];
        stream.read_exact(&mut cmd_buf).await.unwrap();

        // Send feedback for command 1
        let feedback_data = create_feedback_message(1, 0, "OK");
        stream.write_all(&feedback_data).await.unwrap();

        tokio::time::sleep(Duration::from_millis(100)).await;
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect client
    let mut transport = UdsTransport::connect(&socket_path).await.unwrap();

    // Send first command and wait for feedback
    transport
        .send_command(0, 0, "#test", "content", None)
        .await
        .unwrap();
    let feedback = transport.recv_feedback().await.unwrap();
    assert_eq!(feedback.id, 0);
    assert_eq!(feedback.status, 0);

    // Event should have arrived while waiting for second feedback
    // Send second command
    transport
        .send_command(1, 0, "#test", "content2", None)
        .await
        .unwrap();
    let feedback = transport.recv_feedback().await.unwrap();
    // The feedback might be for command 0 or 1 depending on timing
    assert!(
        feedback.id == 0 || feedback.id == 1,
        "Unexpected feedback id: {}",
        feedback.id
    );

    // Now check if the event was queued
    let msg = transport.try_recv_message().await.unwrap();
    assert!(msg.is_some(), "Event should have been queued");

    if let Some(ReceivedMessage::Event(event)) = msg {
        assert_eq!(event.event_type, "submit");
        assert_eq!(event.value, "test");
    } else {
        panic!("Expected Event message, got: {msg:?}");
    }

    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}

fn create_feedback_message(id: u32, status: u8, message: &str) -> Vec<u8> {
    let mut buffer = Vec::new();

    // Message type (0x01 = feedback)
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
    write_string(&mut buffer, "target_id"); // target_id
    write_string(&mut buffer, value);
    write_string(&mut buffer, "{}"); // data

    // Frame it
    let mut framed = Vec::new();
    framed.extend_from_slice(&(buffer.len() as u32).to_le_bytes());
    framed.extend_from_slice(&buffer);
    framed
}
