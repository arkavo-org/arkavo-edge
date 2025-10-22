#![allow(clippy::disallowed_methods)]

use anyhow::Result;
use arkavo_browser::LiveInjector;
use arkavo_router::Router;
use arkavo_ui_generator::planner::UiPlanner;
use arkavo_ui_generator::streaming::StreamingGenerator;
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::page::ScreenshotParams;
use futures::StreamExt;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

/// Test directory for screenshots and artifacts
fn test_output_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test-output");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Integration test: Full E2E flow for calculator UI
#[tokio::test]
#[ignore] // Run with: cargo test --test integration_test -- --ignored --nocapture
async fn test_calculator_ui_generation() -> Result<()> {
    println!("\n🧪 Testing: Calculator UI Generation");

    // Setup browser
    let (browser, mut handler) = Browser::launch(
        BrowserConfig::builder()
            .window_size(1280, 720)
            .build()
            .unwrap(),
    )
    .await?;

    let browser_handle = tokio::spawn(async move { while handler.next().await.is_some() {} });

    let page = browser.new_page("about:blank").await?;
    let injector = LiveInjector::new(page.clone());

    // Step 1: Create router once
    println!("📋 Step 1: Initializing Router...");
    let router = Arc::new(Router::new().await?);

    // Step 2: Plan the UI
    println!("📋 Step 2: Planning UI components...");
    let planner = UiPlanner::new(router.clone());
    let plan = planner.plan("Build a calculator").await?;
    println!("   Generated {} components", plan.parts.len());

    // Step 3: Generate each component
    let generator = StreamingGenerator::new(router)?;

    for (i, part) in plan.parts.iter().enumerate() {
        println!("\n🎨 Step {}: Generating '{}'", i + 2, part.name);

        let mut stream = generator
            .generate_part(&part.name, &part.description, "Build a calculator")
            .await?;

        let mut html = String::new();
        let mut css = String::new();
        let mut js = String::new();

        while let Some(chunk) = stream.recv().await {
            match chunk.chunk_type {
                arkavo_ui_generator::streaming::ChunkType::Html => html.push_str(&chunk.content),
                arkavo_ui_generator::streaming::ChunkType::Css => css.push_str(&chunk.content),
                arkavo_ui_generator::streaming::ChunkType::JavaScript => {
                    js.push_str(&chunk.content)
                }
            }

            if chunk.done {
                break;
            }
        }

        // Inject into browser
        if !css.is_empty() {
            injector.inject_css(&css).await?;
        }
        if !html.is_empty() {
            injector.inject_html(&html).await?;
        }
        if !js.is_empty() {
            injector.inject_javascript(&js).await?;
        }

        // Wait for rendering
        sleep(Duration::from_millis(500)).await;

        // Take screenshot
        let screenshot_path = test_output_dir().join(format!(
            "calculator_step_{:02}_{}.png",
            i + 2,
            sanitize_filename(&part.name)
        ));
        let screenshot = page.screenshot(ScreenshotParams::default()).await?;
        tokio::fs::write(&screenshot_path, screenshot).await?;
        println!("   📸 Screenshot saved: {}", screenshot_path.display());
    }

    // Final screenshot
    let final_screenshot_path = test_output_dir().join("calculator_final.png");
    let final_screenshot = page.screenshot(ScreenshotParams::default()).await?;
    tokio::fs::write(&final_screenshot_path, final_screenshot).await?;
    println!("\n✅ Final screenshot: {}", final_screenshot_path.display());

    browser_handle.abort();
    Ok(())
}

/// Integration test: Dashboard UI with charts
#[tokio::test]
#[ignore] // Run with: cargo test --test integration_test -- --ignored --nocapture
async fn test_dashboard_ui_generation() -> Result<()> {
    println!("\n🧪 Testing: Dashboard UI Generation");

    let (browser, mut handler) = Browser::launch(
        BrowserConfig::builder()
            .window_size(1920, 1080)
            .build()
            .unwrap(),
    )
    .await?;

    let browser_handle = tokio::spawn(async move { while handler.next().await.is_some() {} });

    let page = browser.new_page("about:blank").await?;
    let injector = LiveInjector::new(page.clone());

    // Inject base styles
    injector
        .inject_css(
            r#"
        body {
            margin: 0;
            padding: 20px;
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #0f172a;
            color: #e2e8f0;
        }
        * { box-sizing: border-box; }
    "#,
        )
        .await?;

    let router = Arc::new(Router::new().await?);
    let planner = UiPlanner::new(router.clone());
    let plan = planner
        .plan("Build a dashboard with charts and metrics")
        .await?;
    println!("   Generated {} components", plan.parts.len());

    let generator = StreamingGenerator::new(router)?;

    for (i, part) in plan.parts.iter().enumerate() {
        println!(
            "\n🎨 Generating component {}/{}: {}",
            i + 1,
            plan.parts.len(),
            part.name
        );

        let mut stream = generator
            .generate_part(
                &part.name,
                &part.description,
                "Build a dashboard with charts and metrics",
            )
            .await?;

        let mut html = String::new();
        let mut css = String::new();
        let mut js = String::new();

        while let Some(chunk) = stream.recv().await {
            match chunk.chunk_type {
                arkavo_ui_generator::streaming::ChunkType::Html => html.push_str(&chunk.content),
                arkavo_ui_generator::streaming::ChunkType::Css => css.push_str(&chunk.content),
                arkavo_ui_generator::streaming::ChunkType::JavaScript => {
                    js.push_str(&chunk.content)
                }
            }
            if chunk.done {
                break;
            }
        }

        injector
            .inject_complete_update(
                if html.is_empty() { None } else { Some(&html) },
                if css.is_empty() { None } else { Some(&css) },
                if js.is_empty() { None } else { Some(&js) },
            )
            .await?;

        sleep(Duration::from_millis(500)).await;

        let screenshot_path = test_output_dir().join(format!(
            "dashboard_part_{:02}_{}.png",
            i + 1,
            sanitize_filename(&part.name)
        ));
        let screenshot = page.screenshot(ScreenshotParams::default()).await?;
        tokio::fs::write(&screenshot_path, screenshot).await?;
        println!("   📸 Screenshot: {}", screenshot_path.display());
    }

    let final_path = test_output_dir().join("dashboard_final.png");
    let final_screenshot = page.screenshot(ScreenshotParams::default()).await?;
    tokio::fs::write(&final_path, final_screenshot).await?;
    println!("\n✅ Final dashboard: {}", final_path.display());

    browser_handle.abort();
    Ok(())
}

/// Integration test: Form UI with validation
#[tokio::test]
#[ignore] // Run with: cargo test --test integration_test -- --ignored --nocapture
async fn test_form_ui_generation() -> Result<()> {
    println!("\n🧪 Testing: Form UI Generation");

    let (browser, mut handler) = Browser::launch(
        BrowserConfig::builder()
            .window_size(1024, 768)
            .build()
            .unwrap(),
    )
    .await?;

    let browser_handle = tokio::spawn(async move { while handler.next().await.is_some() {} });

    let page = browser.new_page("about:blank").await?;
    let injector = LiveInjector::new(page.clone());

    let router = Arc::new(Router::new().await?);
    let planner = UiPlanner::new(router.clone());
    let plan = planner
        .plan("Build a user registration form with validation")
        .await?;

    let generator = StreamingGenerator::new(router)?;

    for (i, part) in plan.parts.iter().enumerate() {
        println!("\n🎨 Generating: {}", part.name);

        let mut stream = generator
            .generate_part(
                &part.name,
                &part.description,
                "Build a user registration form with validation",
            )
            .await?;

        let mut html = String::new();
        let mut css = String::new();
        let mut js = String::new();

        while let Some(chunk) = stream.recv().await {
            match chunk.chunk_type {
                arkavo_ui_generator::streaming::ChunkType::Html => html.push_str(&chunk.content),
                arkavo_ui_generator::streaming::ChunkType::Css => css.push_str(&chunk.content),
                arkavo_ui_generator::streaming::ChunkType::JavaScript => {
                    js.push_str(&chunk.content)
                }
            }
            if chunk.done {
                break;
            }
        }

        injector
            .inject_complete_update(
                if html.is_empty() { None } else { Some(&html) },
                if css.is_empty() { None } else { Some(&css) },
                if js.is_empty() { None } else { Some(&js) },
            )
            .await?;

        sleep(Duration::from_millis(500)).await;

        let screenshot_path = test_output_dir().join(format!(
            "form_part_{:02}_{}.png",
            i + 1,
            sanitize_filename(&part.name)
        ));
        let screenshot = page.screenshot(ScreenshotParams::default()).await?;
        tokio::fs::write(&screenshot_path, screenshot).await?;
        println!("   📸 Screenshot: {}", screenshot_path.display());
    }

    browser_handle.abort();
    Ok(())
}

/// Integration test: Bank account UI (from commit example)
#[tokio::test]
#[ignore]
async fn test_bank_portfolio_ui() -> Result<()> {
    println!("\n🧪 Testing: Bank Account & Stock Portfolio UI");

    let (browser, mut handler) = Browser::launch(
        BrowserConfig::builder()
            .window_size(1440, 900)
            .build()
            .unwrap(),
    )
    .await?;

    let browser_handle = tokio::spawn(async move { while handler.next().await.is_some() {} });

    let page = browser.new_page("about:blank").await?;
    let injector = LiveInjector::new(page.clone());

    // Base styles
    injector
        .inject_css(
            r#"
        body {
            margin: 0;
            padding: 20px;
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #f3f4f6;
            color: #1f2937;
        }
    "#,
        )
        .await?;

    let router = Arc::new(Router::new().await?);
    let planner = UiPlanner::new(router.clone());
    let plan = planner
        .plan("Build a bank account and stock portfolio page")
        .await?;

    println!("   Generated {} components", plan.parts.len());

    let generator = StreamingGenerator::new(router)?;

    for (i, part) in plan.parts.iter().enumerate() {
        println!(
            "\n🎨 Component {}/{}: {}",
            i + 1,
            plan.parts.len(),
            part.name
        );

        let mut stream = generator
            .generate_part(
                &part.name,
                &part.description,
                "Build a bank account and stock portfolio page",
            )
            .await?;

        let mut html = String::new();
        let mut css = String::new();
        let mut js = String::new();

        while let Some(chunk) = stream.recv().await {
            match chunk.chunk_type {
                arkavo_ui_generator::streaming::ChunkType::Html => html.push_str(&chunk.content),
                arkavo_ui_generator::streaming::ChunkType::Css => css.push_str(&chunk.content),
                arkavo_ui_generator::streaming::ChunkType::JavaScript => {
                    js.push_str(&chunk.content)
                }
            }
            if chunk.done {
                break;
            }
        }

        injector
            .inject_complete_update(
                if html.is_empty() { None } else { Some(&html) },
                if css.is_empty() { None } else { Some(&css) },
                if js.is_empty() { None } else { Some(&js) },
            )
            .await?;

        sleep(Duration::from_millis(500)).await;

        let screenshot_path = test_output_dir().join(format!(
            "bank_portfolio_{:02}_{}.png",
            i + 1,
            sanitize_filename(&part.name)
        ));
        let screenshot = page.screenshot(ScreenshotParams::default()).await?;
        tokio::fs::write(&screenshot_path, screenshot).await?;
        println!("   📸 Saved: {}", screenshot_path.display());
    }

    let final_path = test_output_dir().join("bank_portfolio_final.png");
    let final_screenshot = page.screenshot(ScreenshotParams::default()).await?;
    tokio::fs::write(&final_path, final_screenshot).await?;
    println!("\n✅ Final result: {}", final_path.display());

    browser_handle.abort();
    Ok(())
}

/// Test incremental UI updates
#[tokio::test]
#[ignore]
async fn test_incremental_updates() -> Result<()> {
    println!("\n🧪 Testing: Incremental UI Updates");

    let (browser, mut handler) = Browser::launch(BrowserConfig::builder().build().unwrap()).await?;
    let browser_handle = tokio::spawn(async move { while handler.next().await.is_some() {} });

    let page = browser.new_page("about:blank").await?;
    let injector = LiveInjector::new(page.clone());

    // Monitor DOM changes
    injector.monitor_changes().await?;

    // Initial component
    injector
        .inject_html(r#"<div id="counter">Count: 0</div>"#)
        .await?;
    sleep(Duration::from_millis(200)).await;

    let screenshot1 = test_output_dir().join("incremental_step_1.png");
    page.screenshot(ScreenshotParams::default())
        .await
        .map(|s| tokio::fs::write(&screenshot1, s))?
        .await?;

    // Update component
    injector
        .inject_html(r#"<div id="counter">Count: 5</div>"#)
        .await?;
    sleep(Duration::from_millis(200)).await;

    let screenshot2 = test_output_dir().join("incremental_step_2.png");
    page.screenshot(ScreenshotParams::default())
        .await
        .map(|s| tokio::fs::write(&screenshot2, s))?
        .await?;

    // Check changes were captured
    let changes = injector.get_captured_changes().await?;
    println!(
        "   Captured {} DOM changes",
        changes.as_array().map(|a| a.len()).unwrap_or(0)
    );

    browser_handle.abort();
    Ok(())
}

/// Helper: Sanitize filename for cross-platform compatibility
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .to_lowercase()
}

#[cfg(test)]
mod screenshot_validation {
    use super::*;

    #[test]
    fn test_screenshot_directory_exists() {
        let dir = test_output_dir();
        assert!(dir.exists(), "Test output directory should exist");
    }

    #[test]
    fn test_filename_sanitization() {
        assert_eq!(
            sanitize_filename("Account Summary Card"),
            "account_summary_card"
        );
        assert_eq!(sanitize_filename("Header (Top)"), "header__top_");
        assert_eq!(sanitize_filename("Portfolio Chart 📊"), "portfolio_chart__");
    }
}
