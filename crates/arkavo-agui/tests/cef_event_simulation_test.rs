#![cfg(feature = "cef-ui")]

use arkavo_agui::renderer::UiRenderer;
use arkavo_agui::renderer::cef_renderer::CefRendererImpl;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_event_simulation() {
    let mut renderer = match CefRendererImpl::new().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping test: CEF renderer not available: {}", e);
            return;
        }
    };

    let html = r#"
        <button id="submit-btn">Submit</button>
        <input id="username" type="text" value="" />
        <div id="output"></div>
    "#;

    renderer.render(html, "", "").await.unwrap();
    sleep(Duration::from_millis(500)).await;

    let events_received = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events_received.clone();

    renderer.set_event_callback(move |event| {
        println!(
            "✓ Event received: {} on {}",
            event.event_type, event.selector
        );
        println!("  Target ID: {}", event.target_id);
        println!("  Value: {}", event.value);
        println!("  Data: {}", event.data);
        events_clone
            .lock()
            .unwrap()
            .push((event.event_type.clone(), event.target_id.clone()));
    });

    renderer
        .add_event_listener("#submit-btn", "click")
        .await
        .unwrap();
    renderer
        .add_event_listener("#username", "input")
        .await
        .unwrap();

    println!("\n=== Event Flow Test ===");
    println!("1. HTML rendered with button and input");
    println!("2. Event listeners attached");
    println!("3. Callback registered");
    println!("\nTo generate actual events:");
    println!("  - User clicks would trigger click events");
    println!("  - Typing in input would trigger input events");
    println!("  - Events queue in window.__arkavoEventQueue");
    println!("  - Poll mechanism would drain queue and send via UDS");
    println!("  - Rust callback would be invoked");
    println!("\nCurrent state: Foundation ready, polling mechanism needed");
    println!("================\n");

    sleep(Duration::from_millis(300)).await;

    let events = events_received.lock().unwrap();
    println!("Events captured in this test: {}", events.len());

    Box::new(renderer).shutdown().await.unwrap();
}
