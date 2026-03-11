//! Anti-pattern store for tracking known failure modes
//!
//! Records recurring failure signatures per (model, category) so the routing
//! layer can penalize models that repeatedly fail in specific ways.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

const MAX_TRACE_IDS: usize = 10;
const DECAY_HALF_LIFE_HOURS: f64 = 24.0;

/// A recorded anti-pattern: a known failure mode for a model+category
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntiPattern {
    pub id: Uuid,
    pub model: String,
    pub category: String,
    /// Signature of the failure (e.g. "hallucinated_tool:read_file")
    pub failure_signature: String,
    pub failure_count: u32,
    /// Decays over time; higher = more relevant
    pub negative_weight: f64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    /// Recent decision trace IDs that led to this failure
    pub decision_trace_ids: Vec<Uuid>,
}

impl AntiPattern {
    fn new(model: String, category: String, failure_signature: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            model,
            category,
            failure_signature,
            failure_count: 1,
            negative_weight: 1.0,
            first_seen: now,
            last_seen: now,
            decision_trace_ids: vec![],
        }
    }

    /// Decay weight based on time elapsed since last_seen
    pub fn decayed_weight(&self) -> f64 {
        let hours_elapsed = (Utc::now() - self.last_seen).num_minutes() as f64 / 60.0;
        self.negative_weight * (0.5_f64).powf(hours_elapsed / DECAY_HALF_LIFE_HOURS)
    }
}

/// Store of anti-patterns indexed by (model, category)
pub struct AntiPatternStore {
    patterns: HashMap<(String, String), Vec<AntiPattern>>,
}

impl Default for AntiPatternStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AntiPatternStore {
    pub fn new() -> Self {
        Self {
            patterns: HashMap::new(),
        }
    }

    /// Record a failure, creating or updating an anti-pattern
    pub fn record_failure(
        &mut self,
        model: &str,
        category: &str,
        failure_signature: &str,
        trace_id: Option<Uuid>,
    ) {
        let key = (model.to_string(), category.to_string());
        let entries = self.patterns.entry(key).or_default();

        if let Some(existing) = entries
            .iter_mut()
            .find(|p| p.failure_signature == failure_signature)
        {
            existing.failure_count += 1;
            existing.negative_weight += 1.0;
            existing.last_seen = Utc::now();
            if let Some(tid) = trace_id {
                if existing.decision_trace_ids.len() >= MAX_TRACE_IDS {
                    existing.decision_trace_ids.remove(0);
                }
                existing.decision_trace_ids.push(tid);
            }
        } else {
            let mut pattern =
                AntiPattern::new(model.into(), category.into(), failure_signature.into());
            if let Some(tid) = trace_id {
                pattern.decision_trace_ids.push(tid);
            }
            entries.push(pattern);
        }
    }

    /// Get the total negative signal for a (model, category) pair
    ///
    /// Returns the sum of decayed weights, useful for penalizing model selection.
    pub fn negative_signal(&self, model: &str, category: &str) -> f64 {
        let key = (model.to_string(), category.to_string());
        self.patterns
            .get(&key)
            .map(|patterns| patterns.iter().map(AntiPattern::decayed_weight).sum())
            .unwrap_or(0.0)
    }

    /// Get all anti-patterns for a model
    pub fn get_for_model(&self, model: &str) -> Vec<&AntiPattern> {
        self.patterns
            .iter()
            .filter(|((m, _), _)| m == model)
            .flat_map(|(_, patterns)| patterns.iter())
            .collect()
    }

    /// Get failure count across all models for a signature
    pub fn total_failures_for_signature(&self, signature: &str) -> u32 {
        self.patterns
            .values()
            .flatten()
            .filter(|p| p.failure_signature == signature)
            .map(|p| p.failure_count)
            .sum()
    }

    /// Total anti-pattern count
    pub fn len(&self) -> usize {
        self.patterns.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Get human-readable warnings for active anti-patterns
    ///
    /// Returns failure signatures with significant decayed weight,
    /// optionally filtered by category.
    pub fn active_warnings(&self, category: Option<&str>) -> Vec<String> {
        const MIN_DECAYED_WEIGHT: f64 = 0.5;

        self.patterns
            .iter()
            .filter(|((_, cat), _)| category.is_none() || category == Some(cat.as_str()))
            .flat_map(|(_, patterns)| patterns.iter())
            .filter(|p| p.decayed_weight() >= MIN_DECAYED_WEIGHT)
            .map(|p| {
                format!(
                    "{} (model={}, seen {}x)",
                    p.failure_signature, p.model, p.failure_count
                )
            })
            .collect()
    }

    /// Classify an error into a failure signature
    pub fn classify_failure(tool_name: &str, error: &str) -> String {
        let lower = error.to_lowercase();
        if lower.contains("hallucin") || lower.contains("does not exist") {
            format!("hallucinated_tool:{tool_name}")
        } else if lower.contains("refused") || lower.contains("i cannot") {
            format!("refusal:{tool_name}")
        } else if lower.contains("timeout") || lower.contains("timed out") {
            format!("timeout:{tool_name}")
        } else if lower.contains("invalid") || lower.contains("malformed") {
            format!("invalid_args:{tool_name}")
        } else {
            format!("error:{tool_name}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_query() {
        let mut store = AntiPatternStore::new();
        store.record_failure("model-a", "code", "hallucinated_tool:read_file", None);
        store.record_failure("model-a", "code", "hallucinated_tool:read_file", None);

        let signal = store.negative_signal("model-a", "code");
        assert!(signal > 1.5, "should accumulate: {signal}");

        assert_eq!(store.negative_signal("model-b", "code"), 0.0);
    }

    #[test]
    fn test_deduplication() {
        let mut store = AntiPatternStore::new();
        store.record_failure("m", "c", "sig1", None);
        store.record_failure("m", "c", "sig1", None);
        store.record_failure("m", "c", "sig2", None);

        assert_eq!(store.len(), 2, "should have 2 unique patterns");

        let patterns = store.get_for_model("m");
        let sig1 = patterns.iter().find(|p| p.failure_signature == "sig1");
        assert_eq!(sig1.unwrap().failure_count, 2);
    }

    #[test]
    fn test_trace_id_tracking() {
        let mut store = AntiPatternStore::new();
        let tid = Uuid::new_v4();
        store.record_failure("m", "c", "sig", Some(tid));

        let patterns = store.get_for_model("m");
        assert_eq!(patterns[0].decision_trace_ids, vec![tid]);
    }

    #[test]
    fn test_trace_id_eviction() {
        let mut store = AntiPatternStore::new();
        for _ in 0..12 {
            store.record_failure("m", "c", "sig", Some(Uuid::new_v4()));
        }
        let patterns = store.get_for_model("m");
        assert_eq!(
            patterns[0].decision_trace_ids.len(),
            MAX_TRACE_IDS,
            "should cap at {MAX_TRACE_IDS}"
        );
    }

    #[test]
    fn test_classify_failure() {
        assert_eq!(
            AntiPatternStore::classify_failure("read_file", "hallucinated path does not exist"),
            "hallucinated_tool:read_file"
        );
        assert_eq!(
            AntiPatternStore::classify_failure("exec", "I cannot do that"),
            "refusal:exec"
        );
        assert_eq!(
            AntiPatternStore::classify_failure("search", "request timed out"),
            "timeout:search"
        );
        assert_eq!(
            AntiPatternStore::classify_failure("write", "invalid JSON argument"),
            "invalid_args:write"
        );
        assert_eq!(
            AntiPatternStore::classify_failure("other", "some random error"),
            "error:other"
        );
    }

    #[test]
    fn test_total_failures_for_signature() {
        let mut store = AntiPatternStore::new();
        store.record_failure("model-a", "code", "refusal:exec", None);
        store.record_failure("model-b", "code", "refusal:exec", None);
        store.record_failure("model-a", "test", "refusal:exec", None);

        assert_eq!(store.total_failures_for_signature("refusal:exec"), 3);
    }

    #[test]
    fn test_empty_store() {
        let store = AntiPatternStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
        assert_eq!(store.negative_signal("any", "any"), 0.0);
    }

    #[test]
    fn test_decay_weight() {
        let mut pattern = AntiPattern::new("m".into(), "c".into(), "sig".into());
        // Weight should be ~1.0 when just created
        assert!((pattern.decayed_weight() - 1.0).abs() < 0.1);

        // Simulate being old (set last_seen far in past)
        pattern.last_seen = Utc::now() - chrono::Duration::hours(48);
        let decayed = pattern.decayed_weight();
        assert!(
            decayed < 0.3,
            "48h old should be heavily decayed: {decayed}"
        );
    }
}
