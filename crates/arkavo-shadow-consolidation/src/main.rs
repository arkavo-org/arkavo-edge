//! Binary entry point for the shadow-consolidation overnight job.

use std::path::PathBuf;

use anyhow::Context;
use arkavo_shadow_consolidation::{Config, execute};
use clap::Parser;

/// Batch existing episode traces to Fable and emit action-named lessons,
/// candidate tightenings, and a cost ledger. Zero runtime contact.
#[derive(Debug, Parser)]
#[command(name = "arkavo-shadow-consolidation", version, about)]
struct Args {
    /// Path to the episode SQLite store (opened read-only).
    #[arg(long, env = "ARKAVO_LEARNING_DB")]
    db: PathBuf,

    /// Directory the artifacts are written to (created if absent).
    #[arg(
        long,
        env = "ARKAVO_SHADOW_OUT",
        default_value = "shadow-consolidation-out"
    )]
    out: PathBuf,

    /// Fable model id.
    #[arg(long, default_value = "claude-fable-5")]
    model: String,

    /// Minimum episodes a category needs before it is consolidated.
    #[arg(long, default_value_t = 3)]
    min_episodes: usize,

    /// `max_tokens` for each Fable completion (thinking is billed within this).
    #[arg(long, default_value_t = 8192)]
    max_tokens: u32,

    /// Reasoning effort for Fable: low | medium | high | max.
    #[arg(long, default_value = "high")]
    effort: String,

    /// Hard ceiling on cumulative run spend in USD. The run stops between
    /// batches once reached, keeps partial outputs, and marks the ledger
    /// budget-exhausted.
    #[arg(long, default_value_t = 25.0)]
    max_run_cost_usd: f64,

    /// Build batches and write prompts without calling Fable (no spend).
    #[arg(long)]
    dry_run: bool,
}

#[tokio::main]
#[allow(clippy::disallowed_methods)] // #[tokio::main] expands to Runtime::block_on
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let dry_run = args.dry_run;
    let cfg = Config {
        db_path: args.db,
        out_dir: args.out,
        model: args.model,
        min_episodes: args.min_episodes,
        max_tokens: args.max_tokens,
        effort: args.effort,
        max_run_cost_usd: args.max_run_cost_usd,
        dry_run,
    };

    let summary = execute(cfg).await.context("shadow consolidation failed")?;

    let mode = if dry_run { " (dry-run)" } else { "" };
    println!(
        "shadow consolidation complete{mode}\n  categories consolidated: {}\n  \
         action-named lessons:    {}\n  candidate tightenings:   {}\n  \
         seed finding cards:      {}\n  rejects:                 {}\n  \
         contract conformance:    {:.1}%\n  total Fable cost:        ${:.4}\n  \
         suggested budget.per_layer[\"consolidation\"].limit_usd: ${:.4}\n  \
         artifacts written to:    {}",
        summary.categories_consolidated,
        summary.lessons,
        summary.proposals,
        summary.findings,
        summary.rejects,
        summary.contract_conformance_rate * 100.0,
        summary.total_cost_usd,
        summary.suggested_per_invocation_limit_usd,
        summary.out_dir.display(),
    );
    if summary.budget_exhausted {
        eprintln!(
            "warning: run-level cost ceiling reached — outputs are partial; \
             see categories_budget_stopped in cost_ledger.json"
        );
    }
    Ok(())
}
