#![cfg(feature = "cef-ui")]

use arkavo_agui::renderer::{create_renderer, RendererType, UiRenderer};
use arkavo_ui_generator::{UiContext, UiGenerationRequest, UiGenerator, UiPreferences};
use std::path::Path;
use tokio::time::Duration;

fn find_latest_screenshot() -> Option<std::path::PathBuf> {
    let tmp_dir = Path::new("/tmp");
    let mut screenshots: Vec<_> = std::fs::read_dir(tmp_dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .map(|name| name.starts_with("arkavo_cef_screenshot_") && name.ends_with(".png"))
                .unwrap_or(false)
        })
        .collect();

    screenshots.sort_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok());

    screenshots.last().map(|entry| entry.path())
}

fn verify_screenshot(path: &Path) -> bool {
    if let Ok(metadata) = std::fs::metadata(path) {
        let size = metadata.len();
        size > 1024 && size < 10_000_000
    } else {
        false
    }
}

#[tokio::test]
async fn test_cef_renderer_startup_shutdown() {
    let renderer = create_renderer(RendererType::Cef).await;

    match renderer {
        Ok(renderer) => {
            assert!(renderer.is_running(), "CEF renderer should be running");

            let shutdown_result = Box::new(renderer).shutdown().await;
            assert!(
                shutdown_result.is_ok(),
                "CEF renderer should shutdown cleanly"
            );
        }
        Err(e) => {
            eprintln!("Skipping test: CEF renderer not available: {}", e);
            return;
        }
    }
}

#[tokio::test]
async fn test_cef_simple_html_rendering() {
    let mut renderer = match create_renderer(RendererType::Cef).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping test: CEF renderer not available: {}", e);
            return;
        }
    };

    let html = r#"
        <div id="test-container">
            <h1>CEF Integration Test</h1>
            <p id="content">Hello from Rust-driven DOM!</p>
        </div>
    "#;

    let css = r#"
        body {
            font-family: system-ui;
            background: #1e1e1e;
            color: #ffffff;
            padding: 20px;
        }
        #test-container {
            max-width: 800px;
            margin: 0 auto;
        }
    "#;

    let result = renderer.render(html, css, "").await;
    assert!(result.is_ok(), "HTML rendering should succeed");

    tokio::time::sleep(Duration::from_millis(1000)).await;

    if let Some(screenshot_path) = find_latest_screenshot() {
        println!("Screenshot found at: {:?}", screenshot_path);
        assert!(
            verify_screenshot(&screenshot_path),
            "Screenshot should be valid"
        );
    } else {
        println!("No screenshot found (may be expected if CEF didn't fully initialize)");
    }

    let shutdown_result = Box::new(renderer).shutdown().await;
    assert!(shutdown_result.is_ok());
}

#[tokio::test]
async fn test_cef_dom_manipulation() {
    let mut renderer = match create_renderer(RendererType::Cef).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping test: CEF renderer not available: {}", e);
            return;
        }
    };

    let html = r#"
        <div id="content">Initial content</div>
        <div id="styled-box">Unstyled box</div>
    "#;

    renderer.render(html, "", "").await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let update_result = renderer
        .update_element("#content", "<p>Updated via DOM command!</p>")
        .await;
    assert!(update_result.is_ok(), "DOM update should succeed");

    let style_result = renderer
        .set_style("#styled-box", "background-color", "blue")
        .await;
    assert!(style_result.is_ok(), "Style update should succeed");

    tokio::time::sleep(Duration::from_millis(500)).await;

    Box::new(renderer).shutdown().await.unwrap();
}

#[tokio::test]
async fn test_end_to_end_ui_generation() {
    let mut renderer = match create_renderer(RendererType::Cef).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping test: CEF renderer not available: {}", e);
            return;
        }
    };

    let generator = match UiGenerator::new().await {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Skipping test: UiGenerator not available: {}", e);
            Box::new(renderer).shutdown().await.ok();
            return;
        }
    };

    let request = UiGenerationRequest {
        user_intent: "Show a simple welcome message with a button".to_string(),
        context: UiContext {
            available_agents: vec!["test-agent".to_string()],
            active_telemetry: vec![],
            current_page: Some("test".to_string()),
        },
        preferences: UiPreferences::default(),
    };

    let generated_ui = generator.generate(request).await.unwrap();

    println!("Generated HTML length: {}", generated_ui.html.len());
    println!("Generated CSS length: {}", generated_ui.css.len());
    println!("Model used: {}", generated_ui.metadata.model_used);
    println!(
        "Generation time: {}ms",
        generated_ui.metadata.generation_time_ms
    );

    let render_result = renderer
        .render(
            &generated_ui.html,
            &generated_ui.css,
            &generated_ui.javascript,
        )
        .await;

    assert!(render_result.is_ok(), "Generated UI should render");

    tokio::time::sleep(Duration::from_secs(1)).await;

    let update_result = renderer
        .update_element("body", "<h1>Dynamic Update Test</h1>")
        .await;
    assert!(update_result.is_ok(), "Dynamic updates should work");

    tokio::time::sleep(Duration::from_millis(500)).await;

    Box::new(renderer).shutdown().await.unwrap();
}

#[tokio::test]
async fn test_cef_multiple_updates() {
    let mut renderer = match create_renderer(RendererType::Cef).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping test: CEF renderer not available: {}", e);
            return;
        }
    };

    let html = r#"
        <div id="counter">0</div>
        <div id="status">Idle</div>
    "#;

    renderer.render(html, "", "").await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    for i in 1..=5 {
        renderer
            .update_element("#counter", &i.to_string())
            .await
            .unwrap();

        renderer
            .update_element("#status", &format!("<span>Update #{}</span>", i))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    renderer
        .set_style("#counter", "font-size", "48px")
        .await
        .unwrap();
    renderer
        .set_style("#counter", "color", "green")
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(500)).await;

    Box::new(renderer).shutdown().await.unwrap();
}

#[tokio::test]
async fn test_cef_event_bridge() {
    use arkavo_agui::renderer::cef_renderer::CefRendererImpl;
    use std::sync::{Arc, Mutex};

    let mut renderer = match CefRendererImpl::new().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping test: CEF renderer not available: {}", e);
            return;
        }
    };

    let html = r#"
        <button id="test-button">Click Me</button>
        <input id="test-input" type="text" value="test" />
    "#;

    renderer.render(html, "", "").await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let events_received = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events_received.clone();

    renderer.set_event_callback(move |event| {
        events_clone.lock().unwrap().push(event.event_type.clone());
        println!("Event received: {:?}", event);
    });

    renderer
        .add_event_listener("#test-button", "click")
        .await
        .unwrap();

    renderer
        .add_event_listener("#test-input", "input")
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(500)).await;

    println!("Event bridge test completed successfully");
    println!("Note: Actual event triggering requires user interaction or JavaScript simulation");

    Box::new(renderer).shutdown().await.unwrap();
}
