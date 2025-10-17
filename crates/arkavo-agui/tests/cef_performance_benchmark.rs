#![cfg(feature = "cef-ui")]

use arkavo_agui::renderer::{RendererType, create_renderer};
use std::time::Instant;
use tokio::time::Duration;

#[tokio::test]
async fn bench_dom_operations() {
    let mut renderer = match create_renderer(RendererType::Cef).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping benchmark: CEF renderer not available: {}", e);
            return;
        }
    };

    let html = r#"
        <div id="benchmark">
            <h1 id="title">Performance Benchmark</h1>
            <p id="content">Initial content</p>
            <div id="styled-box">Box</div>
            <span id="counter">0</span>
        </div>
    "#;

    renderer.render(html, "", "").await.ok();
    tokio::time::sleep(Duration::from_millis(500)).await;

    println!("\n=== CEF DOM Performance Benchmark ===\n");

    let iterations = 10;
    let mut timings = Vec::new();

    for i in 0..iterations {
        let start = Instant::now();
        renderer
            .update_element("#counter", &i.to_string())
            .await
            .ok();
        let elapsed = start.elapsed();
        timings.push(elapsed.as_micros());
    }

    let total: u128 = timings.iter().sum();
    let avg = total / iterations as u128;
    let min = *timings.iter().min().unwrap();
    let max = *timings.iter().max().unwrap();

    println!("ReplaceInnerHTML operations (n={})", iterations);
    println!("  Average: {}μs", avg);
    println!("  Min:     {}μs", min);
    println!("  Max:     {}μs", max);
    println!("  Total:   {}μs", total);

    timings.clear();

    for i in 0..iterations {
        let start = Instant::now();
        renderer
            .set_style(
                "#styled-box",
                "background-color",
                if i % 2 == 0 { "red" } else { "blue" },
            )
            .await
            .ok();
        let elapsed = start.elapsed();
        timings.push(elapsed.as_micros());
    }

    let total: u128 = timings.iter().sum();
    let avg = total / iterations as u128;
    let min = *timings.iter().min().unwrap();
    let max = *timings.iter().max().unwrap();

    println!("\nSetStyle operations (n={})", iterations);
    println!("  Average: {}μs", avg);
    println!("  Min:     {}μs", min);
    println!("  Max:     {}μs", max);
    println!("  Total:   {}μs", total);

    println!("\n=== Benchmark Complete ===\n");

    Box::new(renderer).shutdown().await.ok();
}

#[tokio::test]
async fn bench_sequential_commands() {
    let mut renderer = match create_renderer(RendererType::Cef).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping benchmark: CEF renderer not available: {}", e);
            return;
        }
    };

    let html = r#"<div id="test"></div>"#;
    renderer.render(html, "", "").await.ok();
    tokio::time::sleep(Duration::from_millis(300)).await;

    println!("\n=== Sequential Command Throughput ===\n");

    let commands = 50;
    let start = Instant::now();

    for i in 0..commands {
        renderer
            .update_element("#test", &format!("<span>{}</span>", i))
            .await
            .ok();
    }

    let total_time = start.elapsed();
    let avg_per_command = total_time.as_micros() / commands;
    let commands_per_sec = (1_000_000.0 / avg_per_command as f64) as u32;

    println!("Commands executed: {}", commands);
    println!("Total time: {}ms", total_time.as_millis());
    println!("Avg per command: {}μs", avg_per_command);
    println!("Throughput: {} commands/sec", commands_per_sec);

    println!("\n=== Benchmark Complete ===\n");

    Box::new(renderer).shutdown().await.ok();
}
