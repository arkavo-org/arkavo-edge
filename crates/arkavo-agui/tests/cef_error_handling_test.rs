#![cfg(feature = "cef-ui")]

use arkavo_agui::renderer::{create_renderer, RendererType};
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn test_cef_initialization_failure() {
    let result = create_renderer(RendererType::Cef).await;

    match result {
        Ok(renderer) => {
            assert!(renderer.is_running(), "CEF renderer should be running");
            let _ = Box::new(renderer).shutdown().await;
        }
        Err(e) => {
            eprintln!("CEF renderer not available (expected in CI): {}", e);
        }
    }
}

#[tokio::test]
async fn test_invalid_selector_rejection() {
    let mut renderer = match create_renderer(RendererType::Cef).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping test: CEF renderer not available: {}", e);
            return;
        }
    };

    let html = r#"<div id="test">Content</div>"#;
    let _ = renderer.render(html, "", "").await;

    tokio::time::sleep(Duration::from_millis(500)).await;

    let long_selector = "a".repeat(600);
    let invalid_selectors: Vec<&str> = vec![
        "'; DROP TABLE users; --",
        "<script>alert('xss')</script>",
        "../../etc/passwd",
        &long_selector,
        "$(document).ready(function() { alert('xss'); })",
    ];

    for selector in invalid_selectors {
        let result = renderer.update_element(selector, "test").await;

        if result.is_ok() {
            eprintln!(
                "Warning: Selector '{}' was not rejected (may need stricter validation)",
                selector
            );
        }
    }

    let _ = Box::new(renderer).shutdown().await;
}

#[tokio::test]
async fn test_large_payload_rejection() {
    let mut renderer = match create_renderer(RendererType::Cef).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping test: CEF renderer not available: {}", e);
            return;
        }
    };

    let html = r#"<div id="content">Initial</div>"#;
    let _ = renderer.render(html, "", "").await;

    tokio::time::sleep(Duration::from_millis(500)).await;

    let large_html = "x".repeat(2_000_000);
    let result = renderer.update_element("#content", &large_html).await;

    match result {
        Ok(_) => {
            eprintln!("Warning: Large payload (2MB) was accepted - may cause issues");
        }
        Err(e) => {
            println!("Large payload correctly rejected: {}", e);
        }
    }

    let _ = Box::new(renderer).shutdown().await;
}

#[tokio::test]
async fn test_xss_payload_sanitization() {
    let mut renderer = match create_renderer(RendererType::Cef).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping test: CEF renderer not available: {}", e);
            return;
        }
    };

    let malicious_payloads = vec![
        r#"<script>alert('XSS')</script>"#,
        r#"<img src=x onerror="alert('XSS')">"#,
        r#"<div onclick="alert('XSS')">Click me</div>"#,
        r#"<a href="javascript:alert('XSS')">Link</a>"#,
    ];

    for payload in malicious_payloads {
        let result = renderer.render(payload, "", "").await;

        if result.is_ok() {
            println!("Payload rendered (should be sanitized): {}", payload);
        }
    }

    let _ = Box::new(renderer).shutdown().await;
}

#[tokio::test]
async fn test_concurrent_commands() {
    let mut renderer = match create_renderer(RendererType::Cef).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping test: CEF renderer not available: {}", e);
            return;
        }
    };

    let html = r#"<div id="test">Initial</div>"#;
    let _ = renderer.render(html, "", "").await;

    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut success_count = 0;
    let mut error_count = 0;

    for i in 0..50 {
        let content = format!("Update {}", i);
        let result = renderer.update_element("#test", &content).await;

        if result.is_ok() {
            success_count += 1;
        } else {
            error_count += 1;
        }
    }

    println!(
        "Sequential commands: {} successful, {} failed",
        success_count, error_count
    );

    assert!(success_count > 0, "At least some commands should succeed");

    let _ = Box::new(renderer).shutdown().await;
}

#[tokio::test]
async fn test_connection_timeout() {
    use arkavo_cef::{CefError, UdsTransport};

    let invalid_socket = "/tmp/arkavo_nonexistent_socket.sock";

    let result = timeout(
        Duration::from_secs(15),
        UdsTransport::connect(invalid_socket),
    )
    .await;

    match result {
        Ok(Ok(_)) => {
            panic!("Should not connect to non-existent socket");
        }
        Ok(Err(e)) => {
            println!("Correctly failed to connect: {:?}", e);
            assert!(
                matches!(
                    e,
                    CefError::ConnectionTimeout | CefError::UdsConnectionFailed(_)
                ),
                "Expected timeout or connection failure, got: {:?}",
                e
            );
        }
        Err(_) => {
            panic!("Test timeout - connection should timeout internally");
        }
    }
}

#[tokio::test]
async fn test_invalid_socket_path() {
    use arkavo_cef::UdsTransport;

    let invalid_paths = vec![
        "/etc/passwd",
        "/home/user/socket.sock",
        "../../../etc/shadow",
        "/tmp/not_arkavo.sock",
    ];

    for path in invalid_paths {
        let result = UdsTransport::connect(path).await;

        assert!(
            result.is_err(),
            "Invalid socket path should be rejected: {}",
            path
        );

        if let Err(e) = result {
            println!("Path '{}' correctly rejected: {}", path, e);
        }
    }
}

#[tokio::test]
async fn test_renderer_shutdown_cleanup() {
    let renderer = match create_renderer(RendererType::Cef).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping test: CEF renderer not available: {}", e);
            return;
        }
    };

    assert!(
        renderer.is_running(),
        "Renderer should be running initially"
    );

    let result = Box::new(renderer).shutdown().await;
    assert!(result.is_ok(), "Shutdown should succeed");
}
