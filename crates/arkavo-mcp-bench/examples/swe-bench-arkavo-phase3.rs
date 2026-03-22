use arkavo_mcp_bench::{ArkavoMode, BenchMetrics, BenchSummary, SweBenchInstance, SweBenchTool};
use arkavo_mcp::Tool;
use arkavo_router::Router;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tracing::{error, info, warn};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("🚀 Starting Phase 3: Arkavo-Assisted SWE-bench Validation");
    let overall_start = Instant::now();

    let num_instances = std::env::var("NUM_INSTANCES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(10);

    info!("📊 Configuration:");
    info!("  - Instances: {}", num_instances);
    info!("  - Subset: SWE-bench Lite");
    info!("  - Quality Gate: Enabled (max 3 retries)");
    info!("  - Context: 8000 tokens max");

    let workspace_base = Path::new("/tmp").join(format!("arkavo-phase3-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_base)?;
    info!("  - Workspace: {}", workspace_base.display());

    info!("\n📦 Step 1: Loading SWE-bench instances...");
    let instances = load_instances(num_instances).await?;
    info!("✅ Loaded {} instances", instances.len());

    info!("\n🔧 Step 2: Initializing Arkavo mode...");
    let router = match Router::new().await {
        Ok(r) => {
            info!("✅ Router initialized successfully");
            Arc::new(r)
        }
        Err(e) => {
            error!("❌ Failed to initialize Router: {}", e);
            error!("💡 Make sure GEMINI_API_KEY is set");
            return Err(e.into());
        }
    };

    let arkavo = match ArkavoMode::new(router).await {
        Ok(a) => {
            info!("✅ ArkavoMode initialized with quality gate");
            a
        }
        Err(e) => {
            error!("❌ Failed to initialize ArkavoMode: {}", e);
            return Err(e.into());
        }
    };

    info!("\n🏃 Step 3: Running instances...");
    let mut all_metrics = Vec::new();
    let mut successful = 0;
    let mut failed = 0;

    for (idx, instance) in instances.iter().enumerate() {
        let instance_num = idx + 1;
        info!(
            "\n[{}/{}] Processing: {}",
            instance_num,
            instances.len(),
            instance.instance_id
        );

        let workspace = workspace_base.join(format!("instance-{}", idx));
        fs::create_dir_all(&workspace)?;

        let metrics = match run_single_instance(&arkavo, instance, &workspace).await {
            Ok(m) => {
                if m.resolved {
                    successful += 1;
                    info!("  ✅ RESOLVED in {}ms", m.wall_time_ms);
                } else {
                    failed += 1;
                    info!("  ❌ FAILED in {}ms", m.wall_time_ms);
                    if let Some(ref err) = m.error_message {
                        info!("     Error: {}", err);
                    }
                }

                if let Some(quality_passed) = m.quality_gate_passed {
                    info!(
                        "     Quality Gate: {}",
                        if quality_passed {
                            "✅ PASS"
                        } else {
                            "❌ FAIL"
                        }
                    );
                    info!("     Retries: {}", m.quality_retries);
                }

                m
            }
            Err(e) => {
                failed += 1;
                error!("  ❌ ERROR: {}", e);
                let mut metrics = BenchMetrics::new(instance.instance_id.clone());
                metrics.set_error(format!("Execution error: {}", e));
                metrics
            }
        };

        all_metrics.push(metrics);

        fs::remove_dir_all(&workspace).ok();
    }

    info!("\n📊 Step 4: Generating summary...");
    let summary = BenchSummary::from_metrics(&all_metrics);

    print_summary(&summary, successful, failed);

    let metrics_file = workspace_base.join("phase3-metrics.json");
    save_results(&all_metrics, &summary, &metrics_file)?;
    info!("\n💾 Results saved to: {}", metrics_file.display());

    let total_time = overall_start.elapsed();
    info!("\n⏱️  Total execution time: {:?}", total_time);

    fs::remove_dir_all(&workspace_base).ok();

    info!("\n✨ Phase 3 validation complete!");

    let resolution_rate = summary.resolved_percentage;
    if resolution_rate >= 70.0 {
        info!(
            "🎉 SUCCESS: Resolution rate {}% exceeds 70% target!",
            resolution_rate
        );
    } else {
        warn!(
            "⚠️  BELOW TARGET: Resolution rate {}% is below 70% target",
            resolution_rate
        );
    }

    Ok(())
}

async fn load_instances(limit: usize) -> Result<Vec<SweBenchInstance>, Box<dyn std::error::Error>> {
    let tool = SweBenchTool::new();

    let params = json!({
        "action": "load",
        "subset": "lite",
        "limit": limit
    });

    let result = tool
        .execute(params)
        .await
        .map_err(|e| -> Box<dyn std::error::Error> {
            format!("Failed to load instances: {}", e).into()
        })?;

    let instances: Vec<SweBenchInstance> = serde_json::from_value(result["instances"].clone())
        .map_err(|e| -> Box<dyn std::error::Error> {
            format!("Failed to parse instances: {}", e).into()
        })?;

    Ok(instances)
}

async fn run_single_instance(
    arkavo: &ArkavoMode,
    instance: &SweBenchInstance,
    workspace: &Path,
) -> Result<BenchMetrics, Box<dyn std::error::Error>> {
    info!(
        "  📝 Problem: {}",
        truncate(&instance.problem_statement, 100)
    );
    info!("  📦 Repo: {}", instance.repo);

    clone_repository(&instance.repo, &instance.base_commit, workspace).await?;

    let metrics = arkavo.run_instance(instance, workspace).await?;

    info!("  ⏱️  Wall time: {}ms", metrics.wall_time_ms);
    info!("  🔢 Tokens: {}", metrics.total_tokens);
    info!("  💰 Cost: ${:.4}", metrics.estimated_cost_usd);

    Ok(metrics)
}

async fn clone_repository(
    repo: &str,
    base_commit: &str,
    workspace: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let repo_url = if repo.starts_with("http") {
        repo.to_string()
    } else {
        format!("https://github.com/{}", repo)
    };

    info!("  🔄 Cloning {}...", repo_url);

    let output = tokio::process::Command::new("git")
        .arg("clone")
        .arg(&repo_url)
        .arg(workspace)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to clone repository: {}", stderr).into());
    }

    info!("  📌 Checking out commit {}...", &base_commit[..8]);

    let checkout_output = tokio::process::Command::new("git")
        .arg("checkout")
        .arg(base_commit)
        .current_dir(workspace)
        .output()
        .await?;

    if !checkout_output.status.success() {
        let stderr = String::from_utf8_lossy(&checkout_output.stderr);
        return Err(format!("Failed to checkout commit: {}", stderr).into());
    }

    Ok(())
}

fn print_summary(summary: &BenchSummary, successful: usize, failed: usize) {
    info!("\n╔══════════════════════════════════════════════════════════════╗");
    info!("║                    PHASE 3 RESULTS SUMMARY                   ║");
    info!("╠══════════════════════════════════════════════════════════════╣");
    info!(
        "║  Total Instances:        {:>6}                            ║",
        summary.total_instances
    );
    info!(
        "║  Resolved:               {:>6} ({}%)                      ║",
        successful, summary.resolved_percentage as u32
    );
    info!(
        "║  Failed:                 {:>6}                            ║",
        failed
    );
    info!("║                                                              ║");
    info!(
        "║  Avg Wall Time:          {:>6} ms                         ║",
        summary.avg_wall_time_ms
    );
    info!(
        "║  Total Cost:             ${:>6.2}                         ║",
        summary.total_cost_usd
    );
    info!(
        "║  Avg Cost/Instance:      ${:>6.4}                         ║",
        summary.total_cost_usd / summary.total_instances as f64
    );
    info!("║                                                              ║");
    info!(
        "║  Quality Gate Pass:      {:>6.1}%                          ║",
        summary.quality_gate_pass_rate
    );
    info!(
        "║  Avg Quality Retries:    {:>6.1}                           ║",
        summary.avg_quality_retries
    );
    info!("╚══════════════════════════════════════════════════════════════╝");

    if !summary.issue_type_breakdown.is_empty() {
        info!("\n📋 Issue Type Breakdown:");
        for (issue_type, count) in &summary.issue_type_breakdown {
            info!("  - {:20} {}", issue_type, count);
        }
    }
}

fn save_results(
    metrics: &[BenchMetrics],
    summary: &BenchSummary,
    output_file: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let results = json!({
        "summary": summary,
        "instances": metrics,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    fs::write(output_file, serde_json::to_string_pretty(&results)?)?;

    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}
