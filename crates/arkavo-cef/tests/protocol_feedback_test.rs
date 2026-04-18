use arkavo_cef::UdsTransport;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

/// Test that feedback messages MUST have message type byte 0x01
///
/// This test simulates the bug in C++ SendFeedback (uds_client.cc:254-299)
/// where the message type byte is missing, causing msg_type=0 instead of 0x01.
#[tokio::test]
async fn test_feedback_must_have_message_type_byte() {
    let socket_path = format!("/tmp/arkavo_test_protocol_{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    // Spawn server that sends malformed feedback (missing type byte, like C++ bug)
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        // Read command
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut cmd_buf = vec![0u8; len];
        stream.read_exact(&mut cmd_buf).await.unwrap();

        eprintln!("[TEST SERVER] Received command, sending MALFORMED feedback (no type byte)");

        // Send feedback WITHOUT message type byte (simulates C++ bug)
        let feedback_data = create_malformed_feedback_message(0, 0, "OK");
        stream.write_all(&feedback_data).await.unwrap();

        eprintln!("[TEST SERVER] Sent malformed feedback");

        tokio::time::sleep(Duration::from_millis(100)).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect client
    let mut transport = UdsTransport::connect(&socket_path).await.unwrap();

    // Send command
    transport
        .send_command(0, 0, "#test", "content", None)
        .await
        .unwrap();

    eprintln!("[TEST CLIENT] Command sent, waiting for feedback");

    // Try to receive feedback - should fail or get wrong message type
    let result = transport.recv_feedback().await;

    // The protocol should reject messages without proper type byte
    match result {
        Ok(feedback) => {
            // If it doesn't error, the message type detection is broken
            eprintln!(
                "[TEST CLIENT] Got feedback with id={}, status={}",
                feedback.id, feedback.status
            );
            panic!(
                "Expected protocol error due to missing message type byte, but got valid feedback"
            );
        }
        Err(e) => {
            eprintln!("[TEST CLIENT] Got expected error: {e}");
            // Success - the protocol correctly rejected the malformed message
            assert!(
                e.to_string().contains("Unknown message type")
                    || e.to_string().contains("Protocol")
                    || e.to_string().contains("message type"),
                "Expected protocol error about message type, got: {e}"
            );
        }
    }

    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}

/// Test that properly formed feedback (with type byte) works correctly
#[tokio::test]
async fn test_feedback_with_correct_message_type() {
    let socket_path = format!("/tmp/arkavo_test_protocol_ok_{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    // Spawn server that sends CORRECT feedback (with type byte)
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        // Read command
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut cmd_buf = vec![0u8; len];
        stream.read_exact(&mut cmd_buf).await.unwrap();

        eprintln!("[TEST SERVER] Received command, sending CORRECT feedback");

        // Send feedback WITH message type byte 0x01
        let feedback_data = create_correct_feedback_message(0, 0, "OK");
        stream.write_all(&feedback_data).await.unwrap();

        eprintln!("[TEST SERVER] Sent correct feedback");

        tokio::time::sleep(Duration::from_millis(100)).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect client
    let mut transport = UdsTransport::connect(&socket_path).await.unwrap();

    // Send command
    transport
        .send_command(0, 0, "#test", "content", None)
        .await
        .unwrap();

    eprintln!("[TEST CLIENT] Command sent, waiting for feedback");

    // Receive feedback - should succeed with properly formatted message
    let feedback = transport.recv_feedback().await.unwrap();

    // Verify all fields are correctly deserialized
    assert_eq!(feedback.id, 0);
    assert_eq!(feedback.status, 0);
    assert_eq!(feedback.message, "OK");

    eprintln!(
        "[TEST CLIENT] Got correct feedback: id={}, status={}, message={}",
        feedback.id, feedback.status, feedback.message
    );

    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}

/// Creates a malformed feedback message (missing message type byte)
/// This simulates the bug in C++ uds_client.cc SendFeedback
fn create_malformed_feedback_message(id: u32, status: u8, message: &str) -> Vec<u8> {
    let mut buffer = Vec::new();

    // BUG: Missing message type byte 0x01
    // This is what C++ SendFeedback does incorrectly

    // ID
    buffer.extend_from_slice(&id.to_le_bytes());

    // Status
    buffer.push(status);

    // exec_time_ns
    buffer.extend_from_slice(&0u64.to_le_bytes());

    // Message string (with length prefix)
    let msg_bytes = message.as_bytes();
    buffer.extend_from_slice(&(msg_bytes.len() as u32).to_le_bytes());
    buffer.extend_from_slice(msg_bytes);

    // Frame it
    let mut framed = Vec::new();
    framed.extend_from_slice(&(buffer.len() as u32).to_le_bytes());
    framed.extend_from_slice(&buffer);
    framed
}

/// Creates a correct feedback message (with message type byte 0x01)
/// Matches C++ format from uds_client.cc:254-299
fn create_correct_feedback_message(id: u32, status: u8, message: &str) -> Vec<u8> {
    let mut buffer = Vec::new();

    // CORRECT: Message type byte 0x01 for feedback
    buffer.push(0x01);

    // ID (4 bytes)
    buffer.extend_from_slice(&id.to_le_bytes());

    // Status (1 byte - uint8_t in C++)
    buffer.push(status);

    // exec_time_ns (8 bytes)
    buffer.extend_from_slice(&0u64.to_le_bytes());

    // Message length (4 bytes) then message bytes
    let msg_bytes = message.as_bytes();
    buffer.extend_from_slice(&(msg_bytes.len() as u32).to_le_bytes());
    buffer.extend_from_slice(msg_bytes);

    // Frame it
    let mut framed = Vec::new();
    framed.extend_from_slice(&(buffer.len() as u32).to_le_bytes());
    framed.extend_from_slice(&buffer);
    framed
}
