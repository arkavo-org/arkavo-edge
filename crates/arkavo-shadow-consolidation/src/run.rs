//! Orchestration: read traces, batch to Fable (or dry-run), and write the
//! four artifacts to disk.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Serialize;

use crate::episodes::{CategoryBatch, EpisodeReader, LoadResult};
use crate::fable::{FableClient, FableConfig};
use crate::proposal::{ActionLesson, CostLedger, FindingCard, LedgerEntry, Proposal, RunStats};
use crate::synthesis::{self, ConsolidationOutput};

/// Headroom multiplier applied to the worst observed call cost when
/// recommending the per-invocation consolidation budget.
const BUDGET_HEADROOM: f64 = 1.5;

/// Run configuration, built by the binary from CLI args.
#[derive(Debug, Clone)]
pub struct Config {
    pub db_path: PathBuf,
    pub out_dir: PathBuf,
    pub model: String,
    pub min_episodes: usize,
    pub max_tokens: u32,
    pub effort: String,
    pub dry_run: bool,
}

/// What the run produced, for the binary to print.
#[derive(Debug, Clone)]
pub struct RunSummary {
    pub batches: usize,
    pub lessons: usize,
    pub proposals: usize,
    pub findings: usize,
    pub total_cost_usd: f64,
    pub suggested_per_invocation_limit_usd: f64,
    pub out_dir: PathBuf,
}

/// A category's prompt, recorded in dry-run so the operator can inspect exactly
/// what would be sent before spending.
#[derive(Debug, Serialize)]
struct PromptPreview {
    category: String,
    episode_count: usize,
    system: String,
    user: String,
}

/// Drive a single category: build the prompt, call Fable (unless dry-run), and
/// return the parsed output plus a ledger entry.
async fn consolidate_category(
    client: Option<&FableClient>,
    batch: &CategoryBatch,
) -> anyhow::Result<(ConsolidationOutput, LedgerEntry, PromptPreview)> {
    let episode_count = batch.outcomes.len();
    let user = synthesis::build_user_prompt(batch);
    let preview = PromptPreview {
        category: batch.category.clone(),
        episode_count,
        system: synthesis::SYSTEM_PROMPT.to_string(),
        user: user.clone(),
    };

    let Some(client) = client else {
        // Dry-run: no call, no cost, no parsed output.
        let entry = LedgerEntry {
            category: batch.category.clone(),
            episode_count: episode_count as u32,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            latency_ms: 0,
            lessons: 0,
            tightenings: 0,
        };
        let output = ConsolidationOutput {
            lesson: None,
            proposals: Vec::new(),
        };
        return Ok((output, entry, preview));
    };

    let completion = client
        .complete(synthesis::SYSTEM_PROMPT, &user)
        .await
        .with_context(|| format!("consolidating category {}", batch.category))?;
    let output = synthesis::parse_response(&batch.category, &completion.text, episode_count as u32);
    let entry = LedgerEntry {
        category: batch.category.clone(),
        episode_count: episode_count as u32,
        input_tokens: completion.usage.input_tokens,
        output_tokens: completion.usage.output_tokens,
        cost_usd: completion.usage.cost_usd(),
        latency_ms: completion.latency_ms,
        lessons: u32::from(output.lesson.is_some()),
        tightenings: output.proposals.len() as u32,
    };
    Ok((output, entry, preview))
}

fn write_json<T: Serialize>(dir: &Path, name: &str, value: &T) -> anyhow::Result<()> {
    let path = dir.join(name);
    let json =
        serde_json::to_string_pretty(value).with_context(|| format!("serializing {name}"))?;
    std::fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Execute the full overnight job.
pub async fn execute(cfg: Config) -> anyhow::Result<RunSummary> {
    let reader = EpisodeReader::open(&cfg.db_path).await?;
    let LoadResult {
        batches,
        episodes_total,
        episodes_consolidated,
        categories_total,
        categories_skipped,
    } = reader.load_batches(cfg.min_episodes).await?;

    std::fs::create_dir_all(&cfg.out_dir)
        .with_context(|| format!("creating output dir {}", cfg.out_dir.display()))?;

    let client = if cfg.dry_run {
        None
    } else {
        let fable_cfg =
            FableConfig::from_env(cfg.model.clone(), cfg.max_tokens, cfg.effort.clone())?;
        Some(FableClient::new(fable_cfg)?)
    };

    let mut lessons: Vec<ActionLesson> = Vec::new();
    let mut proposals: Vec<Proposal> = Vec::new();
    let mut findings: Vec<FindingCard> = Vec::new();
    let mut entries: Vec<LedgerEntry> = Vec::new();
    let mut previews: Vec<PromptPreview> = Vec::new();

    for batch in &batches {
        let (output, entry, preview) = consolidate_category(client.as_ref(), batch).await?;
        if let Some(lesson) = output.lesson {
            findings.push(FindingCard::from_lesson(&lesson));
            lessons.push(lesson);
        }
        for proposal in output.proposals {
            findings.push(FindingCard::from_proposal(&proposal));
            proposals.push(proposal);
        }
        entries.push(entry);
        previews.push(preview);
    }

    let ledger = CostLedger::summarize(
        cfg.model.clone(),
        cfg.dry_run,
        RunStats {
            episodes_total,
            episodes_consolidated,
            categories_total,
            categories_skipped,
        },
        entries,
        BUDGET_HEADROOM,
    );

    write_json(&cfg.out_dir, "lessons.json", &lessons)?;
    write_json(&cfg.out_dir, "proposals.json", &proposals)?;
    write_json(&cfg.out_dir, "cost_ledger.json", &ledger)?;
    write_json(&cfg.out_dir, "findings.json", &findings)?;
    if cfg.dry_run {
        write_json(&cfg.out_dir, "prompts.json", &previews)?;
    }

    Ok(RunSummary {
        batches: batches.len(),
        lessons: lessons.len(),
        proposals: proposals.len(),
        findings: findings.len(),
        total_cost_usd: ledger.total_cost_usd,
        suggested_per_invocation_limit_usd: ledger
            .budget_recommendation
            .suggested_per_invocation_limit_usd,
        out_dir: cfg.out_dir.clone(),
    })
}

#[cfg(test)]
mod tests {
    // #[tokio::test] expands to Runtime::block_on; harmless in tests.
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use crate::episodes::ActionOutcome;

    fn batch() -> CategoryBatch {
        CategoryBatch {
            category: "colony".into(),
            outcomes: vec![
                ActionOutcome {
                    actions: vec!["set_priority".into()],
                    success: true,
                    quality: 0.9,
                },
                ActionOutcome {
                    actions: vec!["reset".into()],
                    success: false,
                    quality: 0.1,
                },
                ActionOutcome {
                    actions: vec!["set_priority".into()],
                    success: true,
                    quality: 0.8,
                },
            ],
        }
    }

    #[tokio::test]
    async fn dry_run_makes_no_call_and_zero_cost() {
        let (output, entry, preview) = consolidate_category(None, &batch()).await.unwrap();
        assert!(output.lesson.is_none());
        assert!(output.proposals.is_empty());
        assert!((entry.cost_usd).abs() < f64::EPSILON);
        assert_eq!(entry.episode_count, 3);
        assert_eq!(preview.episode_count, 3);
        assert!(preview.system.contains("consolidation teacher"));
        assert!(preview.user.contains("set_priority"));
    }

    #[tokio::test]
    async fn dry_run_execute_writes_artifacts() {
        // Seed a temp store with one consolidatable category.
        let tmp = tempfile::TempDir::new().unwrap();
        let db = tmp.path().join("learning.db");
        {
            use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
            let opts = SqliteConnectOptions::new()
                .filename(&db)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .unwrap();
            sqlx::query(
                "CREATE TABLE episodes (id TEXT PRIMARY KEY, task_category TEXT NOT NULL, \
                 observation_json TEXT NOT NULL, outcome_json TEXT NOT NULL)",
            )
            .execute(&pool)
            .await
            .unwrap();
            for i in 0..3 {
                sqlx::query(
                    "INSERT INTO episodes (id, task_category, observation_json, outcome_json) \
                     VALUES (?, ?, ?, ?)",
                )
                .bind(format!("id-{i}"))
                .bind("colony")
                .bind(r#"{"tools_used":["observe","set_priority"]}"#)
                .bind(r#"{"success":true,"quality_metrics":{"correctness":0.8}}"#)
                .execute(&pool)
                .await
                .unwrap();
            }
            pool.close().await;
        }

        let out = tmp.path().join("out");
        let cfg = Config {
            db_path: db,
            out_dir: out.clone(),
            model: "claude-fable-5".into(),
            min_episodes: 3,
            max_tokens: 8192,
            effort: "high".into(),
            dry_run: true,
        };
        let summary = execute(cfg).await.unwrap();
        assert_eq!(summary.batches, 1);
        assert!((summary.total_cost_usd).abs() < f64::EPSILON);
        for name in [
            "lessons.json",
            "proposals.json",
            "cost_ledger.json",
            "findings.json",
            "prompts.json",
        ] {
            assert!(out.join(name).exists(), "missing {name}");
        }
        let ledger: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out.join("cost_ledger.json")).unwrap())
                .unwrap();
        assert_eq!(ledger["budget_recommendation"]["layer"], "consolidation");
        assert!(ledger["dry_run"].as_bool().unwrap());
    }
}
