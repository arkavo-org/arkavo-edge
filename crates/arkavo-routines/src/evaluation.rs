//! Matched-workload accounting for the opt-in pilot. Failed tasks and all
//! learning/verification calls contribute to cost per successful task.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measurement {
    pub task_id: String,
    pub phase: Phase,
    pub order_seed: u64,
    pub success: bool,
    pub execution_tokens: u64,
    pub learning_tokens: u64,
    pub verification_tokens: u64,
    pub prompt_tokens: u64,
    pub cached_prompt_tokens: u64,
    pub repeated_actions: u64,
    pub primitive_calls: u64,
    pub latency_ms: u64,
    /// Maximum routing latency within this task, excluding tool execution.
    pub max_routing_us: u64,
    pub policy_violations: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Cold,
    Warm,
    ChangedEnvironment,
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub tasks: usize,
    pub successes: usize,
    pub tokens_per_success: Option<f64>,
    pub prompt_cache_utilization: Option<f64>,
    pub repeated_actions: u128,
    pub primitive_calls: u128,
    pub mean_latency_ms: f64,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub baseline: Summary,
    pub candidate: Summary,
    pub token_reduction: Option<f64>,
    pub passes_pilot_gate: bool,
    pub reasons: Vec<String>,
}

pub fn compare(baseline: &[Measurement], candidate: &[Measurement]) -> Result<Report> {
    let index = |rows: &[Measurement]| -> Result<BTreeMap<_, _>> {
        let mut keys = BTreeMap::new();
        for row in rows {
            if row.task_id.is_empty() || row.cached_prompt_tokens > row.prompt_tokens {
                return Err("Invalid task ID or cache accounting".into());
            }
            let total = u128::from(row.execution_tokens)
                + u128::from(row.learning_tokens)
                + u128::from(row.verification_tokens);
            if u128::from(row.prompt_tokens) > total {
                return Err("Prompt tokens exceed total accounted tokens".into());
            }
            if keys
                .insert(
                    (row.task_id.clone(), row.phase, row.order_seed),
                    row.success,
                )
                .is_some()
            {
                return Err("Duplicate task/phase/order measurement".into());
            }
        }
        Ok(keys)
    };
    let base_index = index(baseline)?;
    let candidate_index = index(candidate)?;
    if baseline.is_empty() || !base_index.keys().eq(candidate_index.keys()) {
        return Err("Compare identical, nonempty task/phase/order sets".into());
    }
    let base = summarize(baseline);
    let cand = summarize(candidate);
    let reduction = base
        .tokens_per_success
        .zip(cand.tokens_per_success)
        .filter(|(b, _)| *b > 0.0)
        .map(|(b, c)| 1.0 - c / b);
    let mut reasons = Vec::new();
    if reduction.is_none_or(|r| r + f64::EPSILON < 0.20) {
        reasons.push("Total tokens per successful task must fall by at least 20%".into());
    }
    for phase in [Phase::Cold, Phase::Warm, Phase::ChangedEnvironment] {
        let b: Vec<_> = baseline.iter().filter(|r| r.phase == phase).collect();
        let c: Vec<_> = candidate.iter().filter(|r| r.phase == phase).collect();
        if b.is_empty() {
            reasons.push(format!("Missing {phase:?} workload"));
        } else if c.iter().filter(|r| r.success).count() < b.iter().filter(|r| r.success).count() {
            reasons.push(format!("Success count regressed in {phase:?} workload"));
        }
    }
    let seeds: std::collections::BTreeSet<_> = baseline.iter().map(|r| r.order_seed).collect();
    if seeds.len() < 2 {
        reasons.push("At least two matched task orders are required".into());
    }
    if baseline
        .iter()
        .chain(candidate)
        .any(|r| r.policy_violations > 0)
    {
        reasons.push("Authorization regression detected in a measured workload".into());
    }
    if candidate.iter().any(|r| r.max_routing_us > 50_000) {
        reasons.push("Routing exceeded 50 ms".into());
    }
    Ok(Report {
        baseline: base,
        candidate: cand,
        token_reduction: reduction,
        passes_pilot_gate: reasons.is_empty(),
        reasons,
    })
}

fn summarize(rows: &[Measurement]) -> Summary {
    let sum = |f: fn(&Measurement) -> u64| rows.iter().map(|r| u128::from(f(r))).sum::<u128>();
    let successes = rows.iter().filter(|r| r.success).count();
    let total =
        sum(|r| r.execution_tokens) + sum(|r| r.learning_tokens) + sum(|r| r.verification_tokens);
    let prompt = sum(|r| r.prompt_tokens);
    Summary {
        tasks: rows.len(),
        successes,
        tokens_per_success: (successes > 0).then(|| total as f64 / successes as f64),
        prompt_cache_utilization: (prompt > 0)
            .then(|| sum(|r| r.cached_prompt_tokens) as f64 / prompt as f64),
        repeated_actions: sum(|r| r.repeated_actions),
        primitive_calls: sum(|r| r.primitive_calls),
        mean_latency_ms: sum(|r| r.latency_ms) as f64 / rows.len() as f64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measurements(tokens: u64) -> Vec<Measurement> {
        [1, 2]
            .into_iter()
            .flat_map(|order_seed| {
                [Phase::Cold, Phase::Warm, Phase::ChangedEnvironment].map(|phase| Measurement {
                    task_id: "task".into(),
                    phase,
                    order_seed,
                    success: true,
                    execution_tokens: tokens,
                    learning_tokens: 0,
                    verification_tokens: 0,
                    prompt_tokens: tokens,
                    cached_prompt_tokens: 0,
                    repeated_actions: 0,
                    primitive_calls: 3,
                    latency_ms: 100,
                    max_routing_us: 10_000,
                    policy_violations: 0,
                })
            })
            .collect()
    }

    #[test]
    fn learning_costs_and_failed_tasks_cannot_be_hidden() {
        let base = measurements(100);
        let mut candidate = measurements(70);
        assert!(compare(&base, &candidate).unwrap().passes_pilot_gate);
        for row in &mut candidate {
            row.learning_tokens = 40;
        }
        let report = compare(&base, &candidate).unwrap();
        assert!(!report.passes_pilot_gate);
        assert_eq!(report.candidate.tokens_per_success, Some(110.0));
        candidate[0].success = false;
        let report = compare(&base, &candidate).unwrap();
        assert_eq!(report.candidate.tokens_per_success, Some(132.0));
        assert!(report.reasons.iter().any(|r| r.contains("Success count")));
    }

    #[test]
    fn rejects_unmatched_duplicate_and_invalid_logs() {
        let base = measurements(100);
        assert!(compare(&[], &[]).is_err());
        assert!(compare(&base, &base[..5]).is_err());
        let mut invalid = base.clone();
        invalid.push(base[0].clone());
        assert!(compare(&invalid, &invalid).is_err());
        invalid = base.clone();
        invalid[0].cached_prompt_tokens = 101;
        assert!(compare(&base, &invalid).is_err());
        invalid = base.clone();
        invalid[0].prompt_tokens = 101;
        assert!(compare(&base, &invalid).is_err());
    }

    #[test]
    fn requires_drift_orders_policy_and_latency_checks() {
        let base = measurements(100);
        let mut candidate = measurements(70);
        candidate[0].policy_violations = 1;
        candidate[0].max_routing_us = 50_001;
        let report = compare(&base, &candidate).unwrap();
        assert_eq!(report.reasons.len(), 2);
        let report = compare(&base[..1], &candidate[..1]).unwrap();
        assert!(report.reasons.iter().any(|r| r.contains("Missing Warm")));
        assert!(report.reasons.iter().any(|r| r.contains("two matched")));
        for row in &mut candidate {
            row.success = false;
        }
        assert!(
            compare(&base, &candidate)
                .unwrap()
                .candidate
                .tokens_per_success
                .is_none()
        );
    }
}
