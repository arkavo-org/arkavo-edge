use arkavo_cef::{CefError, UdsTransport};
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn test_socket_path_validation_rejects_invalid_paths() {
    let invalid_paths = vec![
        "/etc/passwd",
        "/home/user/socket.sock",
        "../../../etc/shadow",
        "/tmp/not_arkavo.sock",
        "/var/run/socket.sock",
        "relative/path.sock",
        "",
        "/tmp/arkavo",
    ];

    for path in invalid_paths {
        let result = UdsTransport::connect(path).await;

        assert!(
            result.is_err(),
            "Path '{path}' should be rejected but was accepted"
        );

        match result {
            Err(CefError::InvalidSocketPath(_)) => {
                println!("✓ Path '{path}' correctly rejected");
            }
            Err(other) => {
                println!("Path '{path}' rejected with error: {other:?}");
            }
            Ok(_) => panic!("Path '{path}' should have been rejected"),
        }
    }
}

#[tokio::test]
async fn test_socket_path_validation_accepts_valid_paths() {
    let valid_paths = vec![
        "/tmp/arkavo_test123.sock",
        "/tmp/arkavo_renderer.sock",
        "/private/tmp/arkavo_session.sock",
    ];

    for path in valid_paths {
        let result = UdsTransport::connect(path).await;

        match result {
            Err(CefError::InvalidSocketPath(msg)) => {
                panic!("Valid path '{path}' rejected: {msg}");
            }
            Err(CefError::UdsConnectionFailed(_) | CefError::ConnectionTimeout) => {
                println!("✓ Path '{path}' passed validation (connection failed as expected)");
            }
            Ok(_) => {
                println!("✓ Path '{path}' connected successfully");
            }
            Err(other) => {
                println!("Path '{path}' failed with: {other:?}");
            }
        }
    }
}

#[tokio::test]
async fn test_socket_path_too_long() {
    let long_path = format!("/tmp/arkavo_{}.sock", "x".repeat(200));

    let result = UdsTransport::connect(&long_path).await;

    assert!(
        result.is_err(),
        "Socket path longer than 100 chars should be rejected"
    );

    if let Err(CefError::InvalidSocketPath(msg)) = result {
        assert!(
            msg.contains("too long"),
            "Error message should mention length: {msg}"
        );
    }
}

#[tokio::test]
async fn test_connection_timeout_on_nonexistent_socket() {
    let socket_path = "/tmp/arkavo_definitely_does_not_exist.sock";

    std::fs::remove_file(socket_path).ok();

    let start = std::time::Instant::now();

    let result = timeout(Duration::from_secs(15), UdsTransport::connect(socket_path)).await;

    let elapsed = start.elapsed();

    println!("Connection attempt took: {elapsed:?}");

    match result {
        Ok(Ok(_)) => panic!("Should not successfully connect to non-existent socket"),
        Ok(Err(CefError::ConnectionTimeout)) => {
            println!("✓ Connection correctly timed out");
            assert!(
                elapsed < Duration::from_secs(12),
                "Timeout should occur within 12 seconds, took {elapsed:?}"
            );
        }
        Ok(Err(CefError::UdsConnectionFailed(_))) => {
            println!("✓ Connection correctly failed immediately");
        }
        Ok(Err(other)) => {
            println!("Connection failed with: {other:?}");
        }
        Err(_) => {
            panic!("Test timeout - internal connection timeout may not be working");
        }
    }
}

#[tokio::test]
async fn test_protocol_serialization_deserialization() {
    use arkavo_cef::protocol::Protocol;

    let command = Protocol::serialize_command(42, 0, "#test", "<div>Hello</div>", None).unwrap();

    assert!(
        !command.is_empty(),
        "Serialized command should not be empty"
    );
    assert!(command.len() > 10, "Command should have reasonable size");

    println!("✓ Command serialized: {} bytes", command.len());
}

#[tokio::test]
async fn test_protocol_framing() {
    use arkavo_cef::protocol::Protocol;
    use bytes::BytesMut;

    let message = b"test message";
    let framed = Protocol::frame_message(message);

    assert_eq!(
        framed.len(),
        message.len() + 4,
        "Frame should add 4-byte length prefix"
    );

    let mut buf = BytesMut::from(&framed[..]);
    let unframed = Protocol::unframe_message(&mut buf).unwrap();

    assert_eq!(
        unframed,
        Some(message.to_vec()),
        "Unframed message should match original"
    );

    println!("✓ Protocol framing works correctly");
}

#[tokio::test]
async fn test_partial_frame_handling() {
    use arkavo_cef::protocol::Protocol;
    use bytes::BytesMut;

    let message = b"test message";
    let framed = Protocol::frame_message(message);

    let partial = &framed[..framed.len() - 2];
    let mut buf = BytesMut::from(partial);

    let result = Protocol::unframe_message(&mut buf).unwrap();
    assert_eq!(result, None, "Partial frame should return None");

    buf.extend_from_slice(&framed[framed.len() - 2..]);

    let result = Protocol::unframe_message(&mut buf).unwrap();
    assert_eq!(
        result,
        Some(message.to_vec()),
        "Complete frame should be extracted"
    );

    println!("✓ Partial frame handling works correctly");
}

#[tokio::test]
async fn test_event_deserialization() {
    use arkavo_cef::protocol::Protocol;

    let event_data: Vec<u8> = vec![
        0x02, 5, 0, 0, 0, b'c', b'l', b'i', b'c', b'k', 7, 0, 0, 0, b'#', b'b', b'u', b't', b't',
        b'o', b'n', 4, 0, 0, 0, b'b', b't', b'n', b'1', 0, 0, 0, 0, 2, 0, 0, 0, b'{', b'}',
    ];

    let result = Protocol::deserialize_event(&event_data);

    assert!(
        result.is_ok(),
        "Valid event should deserialize successfully"
    );

    if let Ok(event) = result {
        assert_eq!(event.event_type, "click");
        assert_eq!(event.selector, "#button");
        assert_eq!(event.target_id, "btn1");
        println!("✓ Event deserialization works correctly");
    }
}

#[tokio::test]
async fn test_invalid_event_deserialization() {
    use arkavo_cef::protocol::Protocol;

    let invalid_events = [
        vec![],
        vec![0x01],
        vec![0x02],
        vec![0x02, 0xFF, 0xFF, 0xFF, 0xFF],
    ];

    for (i, data) in invalid_events.iter().enumerate() {
        let result = Protocol::deserialize_event(data);
        assert!(
            result.is_err(),
            "Invalid event #{i} should fail deserialization"
        );
        println!("✓ Invalid event #{i} correctly rejected");
    }
}

#[tokio::test]
async fn test_sequential_transport_operations() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let socket_path = format!("/tmp/arkavo_sequential_{}.sock", std::process::id());

    std::fs::remove_file(&socket_path).ok();

    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];

        for _ in 0..10 {
            let n = stream.read(&mut buf).await.unwrap();
            stream.write_all(&buf[..n]).await.unwrap();
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut transport = UdsTransport::connect(&socket_path).await.unwrap();

    for i in 0..10 {
        let data = format!("message_{i}").into_bytes();
        let result = transport.send_raw(&data).await;
        assert!(result.is_ok(), "Send operation {i} should succeed");
    }

    println!("✓ Sequential operations completed");

    drop(transport);
    let _ = server.await;

    std::fs::remove_file(&socket_path).ok();
}
