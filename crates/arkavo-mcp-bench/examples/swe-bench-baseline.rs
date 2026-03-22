use arkavo_mcp_bench::SweBenchTool;
use arkavo_mcp::Tool;
use serde_json::json;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("SWE-bench Baseline Benchmark Runner");
    println!("====================================\n");

    let tool = SweBenchTool::new();

    println!("Testing data loading from HuggingFace...\n");

    let subsets = vec![
        ("lite", "SWE-bench Lite (534 instances)"),
        ("verified", "SWE-bench Verified (500 instances)"),
        ("full", "SWE-bench Full (2294 instances)"),
    ];

    for (subset, description) in &subsets {
        println!("📊 Loading {} - {}...", subset, description);
        let start = Instant::now();

        let params = json!({
            "action": "load",
            "subset": subset,
            "limit": 5
        });

        match tool.execute(params).await {
            Ok(result) => {
                let count = result.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
                let elapsed = start.elapsed();
                println!("   ✅ Loaded {} instances in {:?}", count, elapsed);

                if let Some(instances) = result.get("instances").and_then(|v| v.as_array()) {
                    if let Some(first) = instances.first() {
                        if let Some(id) = first.get("instance_id").and_then(|v| v.as_str()) {
                            println!("   📝 First instance: {}", id);
                        }
                    }
                }
            }
            Err(e) => {
                println!("   ❌ Error: {}", e);
            }
        }
        println!();
    }

    println!("\n🎯 Benchmark Execution Test");
    println!("Running 2 instances from lite subset with parallel=2...\n");

    let run_params = json!({
        "action": "run",
        "subset": "lite",
        "limit": 2,
        "parallel": 2,
        "metrics_file": "/tmp/swe-bench-baseline.json"
    });

    let run_start = Instant::now();
    match tool.execute(run_params).await {
        Ok(result) => {
            let elapsed = run_start.elapsed();
            println!("✅ Benchmark completed in {:?}\n", elapsed);

            if let Some(summary) = result.get("summary") {
                println!("📈 Summary:");
                println!(
                    "   Total instances: {}",
                    summary
                        .get("total_instances")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                );
                println!(
                    "   Resolved: {}",
                    summary
                        .get("resolved_count")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                );
                println!(
                    "   Resolution rate: {:.2}%",
                    summary
                        .get("resolved_percentage")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0)
                );
                println!(
                    "   Avg wall time: {}ms",
                    summary
                        .get("avg_wall_time_ms")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                );
                println!(
                    "   Total cost: ${:.4}",
                    summary
                        .get("total_cost_usd")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0)
                );
            }

            println!("\n📁 Metrics saved to /tmp/swe-bench-baseline.json");
        }
        Err(e) => {
            println!("❌ Error running benchmark: {}", e);
        }
    }

    println!("\n🎉 Baseline benchmark test complete!");
    println!("\nNext steps:");
    println!("1. Integrate with Gemini 2.5 Flash for actual problem solving");
    println!("2. Run full benchmark suite with parallel execution");
    println!("3. Compare Gemini Flash vs Pro performance");
    println!("4. Generate comprehensive metrics reports");

    Ok(())
}
