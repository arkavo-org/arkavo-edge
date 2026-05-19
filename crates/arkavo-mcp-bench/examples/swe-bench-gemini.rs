use arkavo_gemini::RestClient;
use arkavo_mcp::Tool;
use arkavo_mcp_bench::SweBenchTool;
use futures::StreamExt;
use serde_json::json;
use std::env;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY environment variable not set");

    println!("🚀 SWE-bench × Gemini Comparative Benchmark");
    println!("============================================\n");

    let bench_tool = SweBenchTool::new();

    println!("📊 Configuration:");
    println!("   Subset: lite (3 instances for comparison)");
    println!("   Models: Flash Latest vs 2.5 Pro");
    println!("   API: Streaming (SSE)\n");

    println!("🔄 Loading SWE-bench Lite instances...");
    let load_params = json!({
        "action": "load",
        "subset": "lite",
        "limit": 3
    });

    let instances_result = bench_tool
        .execute(load_params)
        .await
        .map_err(|e| format!("Failed to load instances: {}", e))?;
    let instances = instances_result
        .get("instances")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "No instances loaded".to_string())?;

    println!("   ✅ Loaded {} instances\n", instances.len());

    let models = vec![
        ("models/gemini-flash-latest", "Gemini Flash Latest"),
        ("models/gemini-3-pro-preview", "Gemini 2.5 Pro"),
    ];

    for (model_id, model_name) in &models {
        println!("\n📈 Benchmarking with {}...", model_name);
        println!("{}", "=".repeat(60));

        let client = RestClient::new(&api_key, *model_id);
        let mut results: Vec<(String, u64, usize, f64)> = Vec::new();

        for (idx, instance) in instances.iter().enumerate() {
            let instance_id = instance
                .get("instance_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let problem = instance
                .get("problem_statement")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            println!(
                "\n  [{}/{}] Instance: {}",
                idx + 1,
                instances.len(),
                instance_id
            );
            println!(
                "  Problem: {}...",
                &problem.chars().take(80).collect::<String>()
            );

            let start = Instant::now();

            let prompt = format!(
                "You are a software engineering expert solving GitHub issues.\n\n\
                 Problem:\n{}\n\n\
                 Provide a git diff patch that solves this issue. \
                 Output ONLY the patch, no explanations.",
                problem
            );

            match generate_solution_streaming(&client, &prompt).await {
                Ok(solution) => {
                    let elapsed = start.elapsed();
                    let tokens_estimate = solution.len() / 4;
                    let cost_estimate = estimate_cost(model_id, tokens_estimate);

                    println!("  ✅ Generated solution in {:?}", elapsed);
                    println!("  📝 Solution length: {} chars", solution.len());
                    println!("  🔢 Est. tokens: {}", tokens_estimate);
                    println!("  💰 Est. cost: ${:.4}", cost_estimate);

                    if solution.len() < 200 {
                        println!("  📄 Full solution:\n{}", solution);
                    } else {
                        println!(
                            "  📄 Preview: {}...",
                            &solution.chars().take(150).collect::<String>()
                        );
                    }

                    results.push((
                        instance_id.to_string(),
                        elapsed.as_millis() as u64,
                        tokens_estimate,
                        cost_estimate,
                    ));
                }
                Err(e) => {
                    println!("  ❌ Error: {}", e);
                    results.push((instance_id.to_string(), 0, 0, 0.0));
                }
            }
        }

        println!("\n\n📊 {} Results Summary", model_name);
        println!("{}", "=".repeat(60));

        let total = results.len();
        let successful = results.iter().filter(|(_, t, _, _)| *t > 0).count();
        let total_time: u64 = results.iter().map(|(_, t, _, _)| t).sum();
        let avg_time = if successful > 0 {
            total_time as f64 / successful as f64
        } else {
            0.0
        };
        let total_tokens: usize = results.iter().map(|(_, _, tk, _)| tk).sum();
        let total_cost: f64 = results.iter().map(|(_, _, _, c)| c).sum();

        println!("  Total instances: {}", total);
        println!("  Successful generations: {}", successful);
        println!(
            "  Success rate: {:.2}%",
            (successful as f64 / total as f64) * 100.0
        );
        println!("  Avg time per instance: {:.2}ms", avg_time);
        println!("  Total tokens: {}", total_tokens);
        println!("  Total cost: ${:.4}", total_cost);
        println!(
            "  Cost per instance: ${:.4}",
            if total > 0 {
                total_cost / total as f64
            } else {
                0.0
            }
        );

        let model_file_name = model_id
            .replace("models/", "")
            .replace('.', "_")
            .replace('-', "_");
        let metrics_file = format!("/tmp/swe-bench-{}.json", model_file_name);
        let metrics_json = serde_json::to_string_pretty(&results)?;
        tokio::fs::write(&metrics_file, metrics_json).await?;
        println!("\n  📁 Results saved to {}", metrics_file);
    }

    println!("\n\n🎉 Comparative Benchmark Complete!");
    println!("\n📊 Quick Comparison:");

    let flash_file = "/tmp/swe-bench-gemini_flash_latest.json";
    let pro_file = "/tmp/swe-bench-gemini_2_5_pro.json";

    if let (Ok(flash_data), Ok(pro_data)) = (
        tokio::fs::read_to_string(flash_file).await,
        tokio::fs::read_to_string(pro_file).await,
    ) {
        let flash_results: Vec<(String, u64, usize, f64)> = serde_json::from_str(&flash_data)?;
        let pro_results: Vec<(String, u64, usize, f64)> = serde_json::from_str(&pro_data)?;

        let flash_avg_time: f64 = flash_results
            .iter()
            .map(|(_, t, _, _)| *t as f64)
            .sum::<f64>()
            / flash_results.len() as f64;
        let pro_avg_time: f64 = pro_results
            .iter()
            .map(|(_, t, _, _)| *t as f64)
            .sum::<f64>()
            / pro_results.len() as f64;

        let flash_total_cost: f64 = flash_results.iter().map(|(_, _, _, c)| c).sum();
        let pro_total_cost: f64 = pro_results.iter().map(|(_, _, _, c)| c).sum();

        println!("\n┌─────────────────────┬─────────────┬─────────────┐");
        println!("│ Metric              │ Flash       │ Pro         │");
        println!("├─────────────────────┼─────────────┼─────────────┤");
        println!(
            "│ Avg Time (ms)       │ {:>11.2} │ {:>11.2} │",
            flash_avg_time, pro_avg_time
        );
        println!(
            "│ Total Cost ($)      │ {:>11.4} │ {:>11.4} │",
            flash_total_cost, pro_total_cost
        );
        println!(
            "│ Speed Advantage     │ {:>10.1}x │ {:>10.1}x │",
            pro_avg_time / flash_avg_time.max(0.1),
            flash_avg_time / pro_avg_time.max(0.1)
        );
        println!(
            "│ Cost Efficiency     │ {:>10.1}x │ {:>10.1}x │",
            pro_total_cost / flash_total_cost.max(0.0001),
            flash_total_cost / pro_total_cost.max(0.0001)
        );
        println!("└─────────────────────┴─────────────┴─────────────┘");
    }

    println!("\nNext Steps:");
    println!("1. Install Docker for full evaluation with test execution");
    println!("2. Run with larger sample (10-50 instances)");
    println!("3. Analyze solution quality and correctness");
    println!("4. Test hybrid Gemini+Gemma routing for cost optimization");

    Ok(())
}

async fn generate_solution_streaming(
    client: &RestClient,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut stream = client.stream_generate_content(prompt, None, None).await?;

    let mut full_text = String::new();

    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(chunk) => {
                if let Some(text) = chunk.text {
                    full_text.push_str(&text);
                }
            }
            Err(e) => {
                return Err(format!("Stream error: {}", e).into());
            }
        }
    }

    if full_text.is_empty() {
        return Err("No response generated".into());
    }

    Ok(full_text)
}

fn estimate_cost(model: &str, tokens: usize) -> f64 {
    let (input_cost, output_cost) = match model {
        "models/gemini-flash-latest" => (0.000075, 0.0003),
        "models/gemini-3-pro-preview" => (0.00125, 0.005),
        _ => (0.0001, 0.0004),
    };

    let avg_cost_per_1k = (input_cost + output_cost) / 2.0;
    (tokens as f64 / 1000.0) * avg_cost_per_1k
}
