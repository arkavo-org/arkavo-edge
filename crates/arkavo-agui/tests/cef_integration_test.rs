#![cfg(feature = "cef-ui")]

use arkavo_agui::renderer::{create_renderer, RendererType, UiRenderer};
use arkavo_ui_generator::{UiContext, UiGenerationRequest, UiGenerator, UiPreferences};
use tokio::time::Duration;

#[tokio::test]
async fn test_cef_renderer_startup_shutdown() {
    let renderer = create_renderer(RendererType::Cef).await;

    match renderer {
        Ok(mut renderer) => {
            assert!(renderer.is_running(), "CEF renderer should be running");

            let shutdown_result = Box::new(renderer).shutdown().await;
            assert!(
                shutdown_result.is_ok(),
                "CEF renderer should shutdown cleanly"
            );
        }
        Err(e) => {
            eprintln!("CEF renderer failed to start: {}", e);
            eprintln!("This test requires ARKAVO_CEF_RENDERER_PATH to be set or renderer binary in target/");
            panic!("CEF renderer startup failed");
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

    tokio::time::sleep(Duration::from_millis(500)).await;

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
