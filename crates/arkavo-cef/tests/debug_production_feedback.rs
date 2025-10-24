/// Quick test to debug the production feedback issue
use arkavo_cef::UdsTransport;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

#[tokio::test]
async fn test_exact_production_feedback_format() {
    let socket_path = format!("/tmp/arkavo_test_prod_fb_{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    // Simulate EXACTLY what C++ SendFeedback sends
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        // Read command
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut cmd_buf = vec![0u8; len];
        stream.read_exact(&mut cmd_buf).await.unwrap();

        eprintln!("[TEST SERVER] Received command, creating feedback like C++");

        // Create feedback EXACTLY like C++ does (from uds_client.cc:262-301)
        let mut buffer: Vec<u8> = Vec::with_capacity(128);

        // Message type 0x01 for feedback (line 266)
        buffer.push(0x01);

        // ID (line 268-269)
        let id: u32 = 0;
        buffer.extend_from_slice(&id.to_le_bytes());

        // Status (line 271-272) - uint8_t in C++, only 1 byte!
        let status: u8 = 0;
        buffer.push(status);

        // exec_time_ns (line 274-275)
        let exec_time_ns: u64 = 210875;
        buffer.extend_from_slice(&exec_time_ns.to_le_bytes());

        // msg_len (placeholder) (line 277-279)
        let msg_len_offset = buffer.len();
        buffer.extend_from_slice(&0u32.to_le_bytes()); // Placeholder

        // Message string (line 281-283)
        let message = "OK";
        buffer.extend_from_slice(message.as_bytes());

        // Calculate and write msg_len (line 285-286)
        let msg_len = message.len() as u32;
        buffer[msg_len_offset..msg_len_offset + 4].copy_from_slice(&msg_len.to_le_bytes());

        eprintln!(
            "[TEST SERVER] Built feedback buffer: total {} bytes",
            buffer.len()
        );
        eprintln!(
            "[TEST SERVER] Buffer contents: {:?}",
            &buffer[..buffer.len().min(30)]
        );

        // Frame it (line 288-299)
        let frame_len = buffer.len() as u32;
        let mut framed = Vec::new();
        framed.extend_from_slice(&frame_len.to_le_bytes());
        framed.extend_from_slice(&buffer);

        stream.write_all(&framed).await.unwrap();
        eprintln!(
            "[TEST SERVER] Sent {} bytes total (4 frame + {} payload)",
            framed.len(),
            buffer.len()
        );

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

    eprintln!("[TEST CLIENT] Waiting for feedback...");

    // Try to receive feedback
    match transport.recv_feedback().await {
        Ok(feedback) => {
            eprintln!(
                "[TEST CLIENT] ✓ Got feedback: id={}, status={}, exec_time_ns={}, message='{}'",
                feedback.id, feedback.status, feedback.exec_time_ns, feedback.message
            );
            assert_eq!(feedback.id, 0);
            assert_eq!(feedback.status, 0);
            assert_eq!(feedback.message, "OK");
        }
        Err(e) => {
            panic!("[TEST CLIENT] Failed to deserialize feedback: {}", e);
        }
    }

    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}
