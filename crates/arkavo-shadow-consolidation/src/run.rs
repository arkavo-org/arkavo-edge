//! Orchestration: read traces, batch to Fable (or dry-run) under a hard
//! run-level cost ceiling, validate output post-hoc, and write the artifacts
//! to disk.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Serialize;

use crate::episodes::{CategoryBatch, EpisodeReader, LoadResult};
use crate::fable::{FableClient, FableConfig, Usage};
use crate::proposal::{
    ActionLesson, Conformance, CostLedger, FindingCard, LedgerEntry, Proposal, RunBudget, RunStats,
};
use crate::synthesis::{self, ConsolidationOutput};
use crate::validate::{self, Reject, RejectKind};

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
    /// Hard ceiling on cumulative run spend; the run stops between batches
    /// once reached and the ledger is marked `budget_exhausted`.
    pub max_run_cost_usd: f64,
    pub dry_run: bool,
}

/// What the run produced, for the binary to print.
#[derive(Debug, Clone)]
pub struct RunSummary {
    /// Categories actually sent to (or previewed for) Fable — excludes any
    /// stopped by the cost ceiling.
    pub categories_consolidated: usize,
    pub lessons: usize,
    pub proposals: usize,
    pub findings: usize,
    pub rejects: usize,
    pub contract_conformance_rate: f64,
    pub budget_exhausted: bool,
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

/// One archived Fable reply, written verbatim to `raw/batch_N.json` for
/// contract-evolution regression.
#[derive(Debug, Serialize)]
struct RawResponse {
    category: String,
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    latency_ms: u64,
    text: String,
}

/// Drive a single category: build the prompt, call Fable (unless dry-run), and
/// return the parsed output, a ledger entry, the prompt preview, and the raw
/// reply for archival.
async fn consolidate_category(
    client: Option<&FableClient>,
    batch: &CategoryBatch,
) -> anyhow::Result<(
    ConsolidationOutput,
    LedgerEntry,
    PromptPreview,
    Option<RawResponse>,
)> {
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
            episode_ids: batch.episode_ids.clone(),
            episode_count: episode_count as u32,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            latency_ms: 0,
            lessons: 0,
            tightenings: 0,
        };
        return Ok((ConsolidationOutput::default(), entry, preview, None));
    };

    let completion = client
        .complete(synthesis::SYSTEM_PROMPT, &user)
        .await
        .with_context(|| format!("consolidating category {}", batch.category))?;
    let output = synthesis::parse_response(&batch.category, &completion.text, episode_count as u32);
    let Usage {
        input_tokens,
        output_tokens,
    } = completion.usage;
    let raw = RawResponse {
        category: batch.category.clone(),
        model: client.model().to_string(),
        input_tokens,
        output_tokens,
        latency_ms: completion.latency_ms,
        text: completion.text,
    };
    let entry = LedgerEntry {
        category: batch.category.clone(),
        episode_ids: batch.episode_ids.clone(),
        episode_count: episode_count as u32,
        input_tokens,
        output_tokens,
        cost_usd: completion.usage.cost_usd(),
        latency_ms: completion.latency_ms,
        lessons: u32::from(output.lesson.is_some()),
        tightenings: output.proposals.len() as u32,
    };
    Ok((output, entry, preview, Some(raw)))
}

fn write_json<T: Serialize>(dir: &Path, name: &str, value: &T) -> anyhow::Result<()> {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let json =
        serde_json::to_string_pretty(value).with_context(|| format!("serializing {name}"))?;
    std::fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Validate one category's parsed output post-hoc, splitting it into accepted
/// artifacts and rejects, and updating the conformance tallies.
struct Accepted {
    lessons: Vec<ActionLesson>,
    proposals: Vec<Proposal>,
    rejects: Vec<Reject>,
}

fn validate_output(
    batch: &CategoryBatch,
    output: ConsolidationOutput,
    conformance: &mut Conformance,
) -> Accepted {
    let mut accepted = Accepted {
        lessons: Vec::new(),
        proposals: Vec::new(),
        rejects: output.rejects,
    };
    // Parse-level rejects are already contract violations — tally them.
    for reject in &accepted.rejects {
        match reject.kind {
            RejectKind::Lesson => conformance.lessons_rejected += 1,
            RejectKind::Tightening => conformance.tightenings_rejected += 1,
            RejectKind::Response => conformance.responses_rejected += 1,
            // Operational failures never reach this path (recorded directly
            // by the run loop) and never count against conformance.
            RejectKind::Call => {}
        }
    }

    let evidence = validate::evidence_actions(&batch.outcomes);

    if let Some(lesson) = output.lesson {
        match validate::lesson_violation(&lesson, &evidence) {
            None => {
                conformance.lessons_accepted += 1;
                accepted.lessons.push(lesson);
            }
            Some(reason) => {
                conformance.lessons_rejected += 1;
                let payload = serde_json::to_value(&lesson).unwrap_or_default();
                accepted.rejects.push(Reject::new(
                    &batch.category,
                    RejectKind::Lesson,
                    reason,
                    payload,
                ));
            }
        }
    }

    for proposal in output.proposals {
        match validate::tightening_violation(&proposal) {
            None => {
                conformance.tightenings_accepted += 1;
                accepted.proposals.push(proposal);
            }
            Some(reason) => {
                conformance.tightenings_rejected += 1;
                let payload = serde_json::to_value(&proposal).unwrap_or_default();
                accepted.rejects.push(Reject::new(
                    &batch.category,
                    RejectKind::Tightening,
                    reason,
                    payload,
                ));
            }
        }
    }

    accepted
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

    // Constructed lazily on the first batch that clears the ceiling check, so
    // a run that can never spend (ceiling 0, or nothing to consolidate) needs
    // no credentials at all.
    let mut client: Option<FableClient> = None;

    let mut lessons: Vec<ActionLesson> = Vec::new();
    let mut proposals: Vec<Proposal> = Vec::new();
    let mut findings: Vec<FindingCard> = Vec::new();
    let mut rejects: Vec<Reject> = Vec::new();
    let mut entries: Vec<LedgerEntry> = Vec::new();
    let mut previews: Vec<PromptPreview> = Vec::new();
    let mut conformance = Conformance::default();
    let mut cumulative_cost = 0.0_f64;
    let mut budget_exhausted = false;
    let mut categories_budget_stopped = 0_u64;

    for (idx, batch) in batches.iter().enumerate() {
        // Hard run-level ceiling: an unattended overnight job with frontier
        // spend must obey the same discipline it is calibrating. Stop between
        // batches, keep partial outputs, and mark the ledger.
        if !cfg.dry_run && cumulative_cost >= cfg.max_run_cost_usd {
            budget_exhausted = true;
            categories_budget_stopped = (batches.len() - idx) as u64;
            break;
        }
        if !cfg.dry_run && client.is_none() {
            let fable_cfg =
                FableConfig::from_env(cfg.model.clone(), cfg.max_tokens, cfg.effort.clone())?;
            client = Some(FableClient::new(fable_cfg)?);
        }

        // One failed call must not abort the run: the ledger has to account
        // for money already spent on earlier batches, so the failure is
        // recorded as a call reject (its own usage never came back) and the
        // run moves on. Call failures are operational, not contract
        // violations — conformance is untouched.
        let (output, entry, preview, raw) = match consolidate_category(client.as_ref(), batch).await
        {
            Ok(parts) => parts,
            Err(e) => {
                rejects.push(Reject::new(
                    &batch.category,
                    RejectKind::Call,
                    format!("{e:#}"),
                    serde_json::Value::Null,
                ));
                continue;
            }
        };
        cumulative_cost += entry.cost_usd;

        if let Some(raw) = raw {
            write_json(&cfg.out_dir, &format!("raw/batch_{idx:03}.json"), &raw)?;
        }

        let accepted = validate_output(batch, output, &mut conformance);
        for lesson in accepted.lessons {
            findings.push(FindingCard::from_lesson(&lesson));
            lessons.push(lesson);
        }
        for proposal in accepted.proposals {
            findings.push(FindingCard::from_proposal(&proposal));
            proposals.push(proposal);
        }
        rejects.extend(accepted.rejects);
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
            categories_budget_stopped,
        },
        conformance,
        RunBudget {
            max_run_cost_usd: cfg.max_run_cost_usd,
            exhausted: budget_exhausted,
        },
        entries,
        BUDGET_HEADROOM,
    );

    write_json(&cfg.out_dir, "lessons.json", &lessons)?;
    write_json(&cfg.out_dir, "proposals.json", &proposals)?;
    write_json(&cfg.out_dir, "cost_ledger.json", &ledger)?;
    write_json(&cfg.out_dir, "findings.json", &findings)?;
    write_json(&cfg.out_dir, "rejects.json", &rejects)?;
    if cfg.dry_run {
        write_json(&cfg.out_dir, "prompts.json", &previews)?;
    }

    Ok(RunSummary {
        categories_consolidated: ledger.categories_consolidated as usize,
        lessons: lessons.len(),
        proposals: proposals.len(),
        findings: findings.len(),
        rejects: rejects.len(),
        contract_conformance_rate: ledger.contract_conformance_rate,
        budget_exhausted,
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
    use crate::proposal::Severity;

    fn batch() -> CategoryBatch {
        CategoryBatch {
            category: "colony".into(),
            episode_ids: vec!["ep-1".into(), "ep-2".into(), "ep-3".into()],
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
        let (output, entry, preview, raw) = consolidate_category(None, &batch()).await.unwrap();
        assert!(output.lesson.is_none());
        assert!(output.proposals.is_empty());
        assert!((entry.cost_usd).abs() < f64::EPSILON);
        assert_eq!(entry.episode_count, 3);
        assert_eq!(entry.episode_ids.len(), 3);
        assert!(raw.is_none());
        assert_eq!(preview.episode_count, 3);
        assert!(preview.system.contains("consolidation teacher"));
        assert!(preview.user.contains("set_priority"));
    }

    #[test]
    fn validate_output_accepts_conformant_and_rejects_violations() {
        let b = batch();
        let mut conformance = Conformance::default();
        let output = ConsolidationOutput {
            // Conformant: names an evidence action.
            lesson: Some(ActionLesson::new(
                "colony".into(),
                "food low".into(),
                "set_priority(Growing)".into(),
                "food recovers".into(),
                0.8,
                3,
            )),
            proposals: vec![
                // Conformant tightening.
                Proposal::new(
                    "colony".into(),
                    "Guard reset".into(),
                    "reset wiped colony".into(),
                    "resets on reconnect".into(),
                    "only reset when TotalReward < -20".into(),
                    Severity::High,
                    0.9,
                    3,
                ),
                // Loosening masquerading as a tightening — must be rejected.
                Proposal::new(
                    "colony".into(),
                    "Free the reset".into(),
                    "resets are fine".into(),
                    "guarded".into(),
                    "relax the reset guard".into(),
                    Severity::Low,
                    0.4,
                    3,
                ),
            ],
            rejects: Vec::new(),
        };

        let accepted = validate_output(&b, output, &mut conformance);
        assert_eq!(accepted.lessons.len(), 1);
        assert_eq!(accepted.proposals.len(), 1);
        assert_eq!(accepted.rejects.len(), 1);
        assert!(accepted.rejects[0].reason.contains("loosens"));
        assert_eq!(conformance.lessons_accepted, 1);
        assert_eq!(conformance.tightenings_accepted, 1);
        assert_eq!(conformance.tightenings_rejected, 1);
        assert!((conformance.rate() - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn validate_output_rejects_lesson_not_in_evidence() {
        let b = batch();
        let mut conformance = Conformance::default();
        let output = ConsolidationOutput {
            lesson: Some(ActionLesson::new(
                "colony".into(),
                "anything".into(),
                "teleport home".into(),
                "safe".into(),
                0.9,
                3,
            )),
            proposals: Vec::new(),
            rejects: Vec::new(),
        };
        let accepted = validate_output(&b, output, &mut conformance);
        assert!(accepted.lessons.is_empty());
        assert_eq!(accepted.rejects.len(), 1);
        assert_eq!(conformance.lessons_rejected, 1);
    }

    async fn seed_store(dir: &Path, categories: &[(&str, usize)]) -> PathBuf {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        let db = dir.join("learning.db");
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
        for (cat, count) in categories {
            for i in 0..*count {
                sqlx::query(
                    "INSERT INTO episodes (id, task_category, observation_json, outcome_json) \
                     VALUES (?, ?, ?, ?)",
                )
                .bind(format!("{cat}-{i}"))
                .bind(*cat)
                .bind(r#"{"tools_used":["observe","set_priority"]}"#)
                .bind(r#"{"success":true,"quality_metrics":{"correctness":0.8}}"#)
                .execute(&pool)
                .await
                .unwrap();
            }
        }
        pool.close().await;
        db
    }

    #[tokio::test]
    async fn dry_run_execute_writes_artifacts() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = seed_store(tmp.path(), &[("colony", 3)]).await;

        let out = tmp.path().join("out");
        let cfg = Config {
            db_path: db,
            out_dir: out.clone(),
            model: "claude-fable-5".into(),
            min_episodes: 3,
            max_tokens: 8192,
            effort: "high".into(),
            max_run_cost_usd: 25.0,
            dry_run: true,
        };
        let summary = execute(cfg).await.unwrap();
        assert_eq!(summary.categories_consolidated, 1);
        assert!((summary.total_cost_usd).abs() < f64::EPSILON);
        assert!(!summary.budget_exhausted);
        for name in [
            "lessons.json",
            "proposals.json",
            "cost_ledger.json",
            "findings.json",
            "rejects.json",
            "prompts.json",
        ] {
            assert!(out.join(name).exists(), "missing {name}");
        }
        let ledger: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out.join("cost_ledger.json")).unwrap())
                .unwrap();
        assert_eq!(ledger["budget_recommendation"]["layer"], "consolidation");
        assert!(ledger["dry_run"].as_bool().unwrap());
        assert!(!ledger["run_budget"]["exhausted"].as_bool().unwrap());
        // Batch→episode linkage preserved for backfill.
        assert_eq!(
            ledger["per_call"][0]["episode_ids"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert!((ledger["contract_conformance_rate"].as_f64().unwrap() - 1.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn ceiling_at_or_below_zero_stops_before_first_live_call() {
        // With a zero ceiling and dry_run=false, the run must stop before
        // batch 0 — the ceiling check fires before the client is ever
        // constructed, so no credentials and no network are needed.
        let tmp = tempfile::TempDir::new().unwrap();
        let db = seed_store(tmp.path(), &[("colony", 3), ("nav", 3)]).await;
        let out = tmp.path().join("out");
        let cfg = Config {
            db_path: db,
            out_dir: out.clone(),
            model: "claude-fable-5".into(),
            min_episodes: 3,
            max_tokens: 8192,
            effort: "high".into(),
            max_run_cost_usd: 0.0,
            dry_run: false,
        };
        let summary = execute(cfg).await.unwrap();
        assert!(summary.budget_exhausted);
        assert_eq!(summary.categories_consolidated, 0);
        assert!((summary.total_cost_usd).abs() < f64::EPSILON);
        assert!(summary.total_cost_usd.is_sign_positive());
        // Partial outputs still written.
        assert!(out.join("cost_ledger.json").exists());
        let ledger: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out.join("cost_ledger.json")).unwrap())
                .unwrap();
        assert!(ledger["run_budget"]["exhausted"].as_bool().unwrap());
        assert_eq!(ledger["categories_budget_stopped"].as_u64().unwrap(), 2);
        assert_eq!(ledger["fable_calls"].as_u64().unwrap(), 0);
    }
}
