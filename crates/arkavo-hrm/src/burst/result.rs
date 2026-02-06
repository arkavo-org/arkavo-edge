//! Burst execution results and continuation hints

use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

/// Result from burst execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BurstResult {
    /// Contract ID this result is for
    pub contract_id: Uuid,
    /// Steps taken during execution
    pub steps_taken: u32,
    /// Tokens consumed
    pub tokens_used: u64,
    /// Cost incurred in USD
    pub cost_usd: f64,
    /// Execution duration
    #[serde(with = "duration_serde")]
    pub duration: Duration,
    /// Whether the work was completed
    pub completed: bool,
    /// Output data (if any)
    pub output: Option<serde_json::Value>,
    /// Continuation hint for "almost done" scenarios
    pub continuation_hint: Option<ContinuationHint>,
    /// Error message if execution failed
    pub error: Option<String>,
    /// Model used for execution
    pub model_used: Option<String>,
    /// Agent used for execution
    pub agent_used: Option<String>,
}

impl BurstResult {
    /// Create a successful result
    pub fn success(contract_id: Uuid, output: serde_json::Value) -> Self {
        Self {
            contract_id,
            steps_taken: 0,
            tokens_used: 0,
            cost_usd: 0.0,
            duration: Duration::ZERO,
            completed: true,
            output: Some(output),
            continuation_hint: None,
            error: None,
            model_used: None,
            agent_used: None,
        }
    }

    /// Create a failed result
    pub fn failure(contract_id: Uuid, error: String) -> Self {
        Self {
            contract_id,
            steps_taken: 0,
            tokens_used: 0,
            cost_usd: 0.0,
            duration: Duration::ZERO,
            completed: false,
            output: None,
            continuation_hint: None,
            error: Some(error),
            model_used: None,
            agent_used: None,
        }
    }

    /// Create a partial result that needs continuation
    pub fn partial(contract_id: Uuid, hint: ContinuationHint) -> Self {
        Self {
            contract_id,
            steps_taken: 0,
            tokens_used: 0,
            cost_usd: 0.0,
            duration: Duration::ZERO,
            completed: false,
            output: None,
            continuation_hint: Some(hint),
            error: None,
            model_used: None,
            agent_used: None,
        }
    }

    /// Set metrics
    pub fn with_metrics(mut self, steps: u32, tokens: u64, cost: f64, duration: Duration) -> Self {
        self.steps_taken = steps;
        self.tokens_used = tokens;
        self.cost_usd = cost;
        self.duration = duration;
        self
    }

    /// Set execution context
    pub fn with_execution_context(mut self, model: String, agent: String) -> Self {
        self.model_used = Some(model);
        self.agent_used = Some(agent);
        self
    }

    /// Check if this result indicates success
    pub fn is_success(&self) -> bool {
        self.completed && self.error.is_none()
    }

    /// Check if this result indicates failure
    pub fn is_failure(&self) -> bool {
        self.error.is_some()
    }

    /// Check if this result needs continuation
    pub fn needs_continuation(&self) -> bool {
        !self.completed && self.error.is_none() && self.continuation_hint.is_some()
    }
}

/// Hint for requesting budget extension ("Almost Done" problem)
///
/// When a burst is near completion but exhausted its budget, it can
/// provide a ContinuationHint to request a small extension rather than
/// starting over completely.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinuationHint {
    /// Estimated additional steps needed
    pub estimated_steps: u32,
    /// Estimated additional tokens
    pub estimated_tokens: u64,
    /// Estimated additional cost in USD
    pub estimated_cost_usd: f64,
    /// Estimated additional time
    #[serde(with = "duration_serde")]
    pub estimated_time: Duration,
    /// Confidence in the estimate (0.0-1.0)
    pub confidence: f64,
    /// Human-readable reason for needing extension
    pub reason: String,
    /// Progress completed so far (0.0-1.0)
    pub progress: f64,
    /// Partial output that can be continued
    pub partial_output: Option<serde_json::Value>,
}

impl ContinuationHint {
    /// Create a new continuation hint
    pub fn new(
        estimated_steps: u32,
        confidence: f64,
        progress: f64,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            estimated_steps,
            estimated_tokens: (estimated_steps as u64) * 500, // Rough estimate
            estimated_cost_usd: (estimated_steps as f64) * 0.01,
            estimated_time: Duration::from_secs(estimated_steps as u64 * 10),
            confidence,
            reason: reason.into(),
            progress,
            partial_output: None,
        }
    }
}

/// Serde support for Duration
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_millis().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_burst_result_success() {
        let contract_id = Uuid::new_v4();
        let output = serde_json::json!({"result": "done"});
        let result = BurstResult::success(contract_id, output);

        assert!(result.is_success());
        assert!(!result.is_failure());
        assert!(!result.needs_continuation());
    }

    #[test]
    fn test_burst_result_failure() {
        let contract_id = Uuid::new_v4();
        let result = BurstResult::failure(contract_id, "Something went wrong".to_string());

        assert!(!result.is_success());
        assert!(result.is_failure());
        assert!(!result.needs_continuation());
    }

    #[test]
    fn test_burst_result_partial() {
        let contract_id = Uuid::new_v4();
        let hint = ContinuationHint::new(2, 0.9, 0.8, "Almost done");
        let result = BurstResult::partial(contract_id, hint);

        assert!(!result.is_success());
        assert!(!result.is_failure());
        assert!(result.needs_continuation());
    }
}
