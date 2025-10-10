use arkavo_browser::{BrowserTool, LiveInjector};
use arkavo_ui_generator::incremental::IncrementalUiBuilder;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🚀 Starting Incremental UI Generation Demo");
    println!("📋 Prompt: Build a bank account and stock portfolio page");
    println!();

    let builder = IncrementalUiBuilder::new(
        "Build a bank account and stock portfolio page".to_string()
    );

    let components = builder.plan_components().await?;
    println!("📊 Planned {} components:", components.len());
    for (i, comp) in components.iter().enumerate() {
        println!("  {}. {}", i + 1, comp);
    }
    println!();

    println!("🌐 Launching Chrome...");
    let browser = BrowserTool::new().await?;
    let page = browser.get_browser()
        .new_page("about:blank")
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create page: {}", e))?;

    let injector = LiveInjector::new(page);

    println!("✨ Injecting base styles...");
    let base_css = r#"
        body {
            margin: 0;
            padding: 20px;
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #f3f4f6;
            color: #1f2937;
        }

        * {
            box-sizing: border-box;
        }
    "#;
    injector.inject_css(base_css).await?;

    println!();
    println!("🎨 Generating UI incrementally (press Ctrl+C to stop and modify)...");
    println!();

    let mut part_num = 1;
    while let Some(part) = builder.generate_next_part().await? {
        println!("⚡ Part {}/{}: {}", part_num, components.len(), part.name);
        println!("   Description: {}", part.description);

        injector.inject_html(&part.html).await?;
        injector.inject_css(&part.css).await?;

        if !part.javascript.is_empty() {
            injector.inject_javascript(&part.javascript).await?;
        }

        println!("   ✓ Injected {} lines of HTML", part.html.lines().count());
        println!("   ✓ Injected {} lines of CSS", part.css.lines().count());
        if !part.javascript.is_empty() {
            println!("   ✓ Injected {} lines of JavaScript", part.javascript.lines().count());
        }
        println!();

        part_num += 1;

        sleep(Duration::from_secs(2)).await;
    }

    println!("🎉 UI Generation Complete!");
    println!("🔧 You can now modify the UI in Chrome DevTools");
    println!("💡 Changes will be detected and sent to the coding agent for refinement");
    println!();
    println!("Press Enter to exit...");

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

    Ok(())
}
