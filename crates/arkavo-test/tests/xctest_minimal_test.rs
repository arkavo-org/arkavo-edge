//! Minimal test for XCTest socket communication

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::Duration;

#[test]
#[cfg(target_os = "macos")]
fn test_basic_unix_socket() {
    println!("\n=== Basic Unix Socket Test ===\n");

    // Create a simple socket path
    let socket_path = "/tmp/test-socket.sock";

    // Remove old socket if exists
    let _ = std::fs::remove_file(socket_path);

    // Start a simple server in a thread
    let server_path = socket_path.to_string();
    let server_thread = thread::spawn(move || {
        use std::os::unix::net::UnixListener;

        println!("Server: Creating listener at {}", server_path);
        let listener = UnixListener::bind(&server_path).expect("Failed to bind");
        println!("Server: Listening for connections");

        if let Ok((mut stream, _)) = listener.accept() {
            println!("Server: Client connected!");

            // Send ready signal
            stream
                .write_all(b"[READY]\n")
                .expect("Failed to send ready");
            stream.flush().expect("Failed to flush");
            println!("Server: Sent ready signal");

            // Read command
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            if reader.read_line(&mut line).is_ok() {
                println!("Server: Received: {}", line.trim());

                // Send response
                let response = format!("{{\"success\":true,\"data\":\"{}\"}}\n", line.trim());
                stream
                    .write_all(response.as_bytes())
                    .expect("Failed to send response");
                stream.flush().expect("Failed to flush");
                println!("Server: Sent response");
            }
        }
    });

    // Give server time to start
    thread::sleep(Duration::from_millis(500));

    // Connect as client
    println!("\nClient: Connecting to {}", socket_path);
    let stream = UnixStream::connect(socket_path).expect("Failed to connect");
    println!("Client: Connected!");

    // Clone for reading and writing
    let mut write_stream = stream.try_clone().expect("Failed to clone stream");
    let mut reader = BufReader::new(stream);

    // Read ready signal
    let mut line = String::new();
    reader.read_line(&mut line).expect("Failed to read ready");
    println!("Client: Received: {}", line.trim());
    assert_eq!(line.trim(), "[READY]");

    // Send command
    write_stream
        .write_all(b"TEST_COMMAND\n")
        .expect("Failed to send command");
    write_stream.flush().expect("Failed to flush");
    println!("Client: Sent command");

    // Read response
    line.clear();
    reader
        .read_line(&mut line)
        .expect("Failed to read response");
    println!("Client: Received response: {}", line.trim());

    // Verify response
    assert!(line.contains("success"));
    assert!(line.contains("TEST_COMMAND"));

    // Cleanup
    server_thread.join().unwrap();
    let _ = std::fs::remove_file(socket_path);

    println!("\n✅ Basic Unix socket test passed!\n");
}
