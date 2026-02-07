#![allow(clippy::disallowed_methods)]

use arkavo_cef::UdsTransport;
/// Test shutdown detection when the CEF process exits
use std::time::Duration;
use tokio::net::UnixListener;

#[tokio::test]
async fn test_detect_process_exit_via_connection_close() {
    let socket_path = format!("/tmp/arkavo_test_shutdown_{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).unwrap();

    // Server simulates CEF process that exits
    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();

        eprintln!("[TEST SERVER] Connected");

        // Simulate normal operation
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Simulate window close - drop connection (like exit(0) does)
        eprintln!("[TEST SERVER] Simulating process exit (dropping connection)");
        drop(stream);
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut transport = UdsTransport::connect(&socket_path).await.unwrap();

    eprintln!("[TEST CLIENT] Connected, polling for messages");

    // Poll for messages - should eventually detect connection closed
    let mut detected_close = false;
    for i in 1..=20 {
        eprintln!("[TEST CLIENT] Poll #{i}");

        match transport.try_recv_message().await {
            Ok(Some(msg)) => {
                eprintln!("[TEST CLIENT] Got message: {msg:?}");
            }
            Ok(None) => {
                eprintln!("[TEST CLIENT] No message");
            }
            Err(e) => {
                eprintln!("[TEST CLIENT] Error: {e}");
                if e.to_string().contains("Connection closed") {
                    eprintln!("[TEST CLIENT] ✓ Detected connection closed!");
                    detected_close = true;
                    break;
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert!(
        detected_close,
        "Should detect connection closed when process exits"
    );
    server_task.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}
