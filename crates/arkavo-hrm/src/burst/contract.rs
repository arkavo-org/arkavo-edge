//! Burst contract definition and management

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

/// Current state of a burst execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BurstState {
    /// Contract is active and execution is allowed
    #[default]
    Active,
    /// Execution completed within bounds
    Completed,
    /// Budget was exhausted (steps, tokens, or cost)
    Exhausted,
    /// Wall time expired
    Expired,
    /// Contract was cancelled
    Cancelled,
}

impl BurstState {
    /// Check if this state is terminal
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Active)
    }

    /// Check if work can continue
    pub fn can_continue(&self) -> bool {
        matches!(self, Self::Active)
    }
}

/// Strategy for passing context between bursts
///
/// Controls how much context from previous subtasks is passed to the next,
/// preventing context bloat while maintaining necessary continuity.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum ContextStrategy {
    /// Pass all context from previous bursts (risk of bloat)
    Full,
    /// Generate an LLM summary of previous context
    SummaryOnly,
    /// Keep only the last N messages/items
    LastNMessages(usize),
    /// Pass only artifact IDs, fetch content on demand
    #[default]
    ArtifactReference,
}

impl ContextStrategy {
    /// Estimate token overhead for this strategy
    pub fn estimated_overhead_tokens(&self) -> u64 {
        match self {
            Self::Full => 5000,
            Self::SummaryOnly => 500,
            Self::LastNMessages(n) => (*n as u64) * 200,
            Self::ArtifactReference => 100,
        }
    }
}

/// A bounded execution contract
///
/// Defines the resource limits for a single burst of work. The executor
/// must respect these limits and terminate when any limit is reached.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BurstContract {
    /// Contract identifier
    pub id: Uuid,
    /// Associated subtask
    pub subtask_id: Uuid,
    /// Maximum execution steps (tool calls, reasoning steps)
    pub max_steps: u32,
    /// Maximum wall time for this burst
    #[serde(with = "duration_serde")]
    pub max_wall_time: Duration,
    /// Maximum tokens to consume
    pub max_tokens: u64,
    /// Maximum cost in USD
    pub max_cost_usd: f64,
    /// When the contract was issued
    pub issued_at: DateTime<Utc>,
    /// Contract expiry time
    pub expires_at: DateTime<Utc>,
    /// Current state
    pub state: BurstState,
    /// Context strategy for this burst
    pub context_strategy: ContextStrategy,
    /// Whether extensions are allowed for "almost done" scenarios
    pub allow_extensions: bool,
    /// Maximum number of extensions allowed
    pub max_extensions: u32,
    /// Number of extensions already granted
    pub extensions_granted: u32,
}

impl BurstContract {
    /// Create a new burst contract with default settings
    pub fn new(subtask_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            subtask_id,
            max_steps: 3,
            max_wall_time: Duration::from_secs(45),
            max_tokens: 10_000,
            max_cost_usd: 0.50,
            issued_at: now,
            expires_at: now + chrono::Duration::seconds(60),
            state: BurstState::Active,
            context_strategy: ContextStrategy::default(),
            allow_extensions: true,
            max_extensions: 2,
            extensions_granted: 0,
        }
    }

    /// Create a contract with custom limits
    pub fn with_limits(
        subtask_id: Uuid,
        max_steps: u32,
        max_wall_time: Duration,
        max_tokens: u64,
        max_cost_usd: f64,
    ) -> Self {
        let now = Utc::now();
        let expiry = now
            + chrono::Duration::from_std(max_wall_time + Duration::from_secs(15))
                .unwrap_or(chrono::Duration::seconds(60));

        Self {
            id: Uuid::new_v4(),
            subtask_id,
            max_steps,
            max_wall_time,
            max_tokens,
            max_cost_usd,
            issued_at: now,
            expires_at: expiry,
            state: BurstState::Active,
            context_strategy: ContextStrategy::default(),
            allow_extensions: true,
            max_extensions: 2,
            extensions_granted: 0,
        }
    }

    /// Set the context strategy
    pub fn with_context_strategy(mut self, strategy: ContextStrategy) -> Self {
        self.context_strategy = strategy;
        self
    }

    /// Disable extensions
    pub fn without_extensions(mut self) -> Self {
        self.allow_extensions = false;
        self
    }

    /// Check if the contract is still valid
    pub fn is_valid(&self) -> bool {
        self.state == BurstState::Active && Utc::now() < self.expires_at
    }

    /// Check if the contract has expired
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    /// Check if an extension can be granted
    pub fn can_extend(&self) -> bool {
        self.allow_extensions && self.extensions_granted < self.max_extensions
    }

    /// Grant an extension, returning a new contract
    pub fn extend(&self, additional_steps: u32, additional_time: Duration) -> Option<Self> {
        if !self.can_extend() {
            return None;
        }

        let now = Utc::now();
        Some(Self {
            id: Uuid::new_v4(),
            subtask_id: self.subtask_id,
            max_steps: additional_steps,
            max_wall_time: additional_time,
            max_tokens: self.max_tokens / 2, // Reduce token budget for extensions
            max_cost_usd: self.max_cost_usd / 2.0,
            issued_at: now,
            expires_at: now
                + chrono::Duration::from_std(additional_time + Duration::from_secs(15))
                    .unwrap_or(chrono::Duration::seconds(60)),
            state: BurstState::Active,
            context_strategy: self.context_strategy.clone(),
            allow_extensions: self.allow_extensions,
            max_extensions: self.max_extensions,
            extensions_granted: self.extensions_granted + 1,
        })
    }

    /// Remaining time until expiry
    pub fn remaining_time(&self) -> Duration {
        let now = Utc::now();
        if now >= self.expires_at {
            Duration::ZERO
        } else {
            (self.expires_at - now).to_std().unwrap_or(Duration::ZERO)
        }
    }

    /// Mark the contract as completed
    pub fn complete(&mut self) {
        self.state = BurstState::Completed;
    }

    /// Mark the contract as exhausted
    pub fn exhaust(&mut self) {
        self.state = BurstState::Exhausted;
    }

    /// Mark the contract as expired
    pub fn expire(&mut self) {
        self.state = BurstState::Expired;
    }

    /// Mark the contract as cancelled
    pub fn cancel(&mut self) {
        self.state = BurstState::Cancelled;
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
    fn test_contract_creation() {
        let subtask_id = Uuid::new_v4();
        let contract = BurstContract::new(subtask_id);

        assert_eq!(contract.subtask_id, subtask_id);
        assert!(contract.is_valid());
        assert!(!contract.is_expired());
        assert_eq!(contract.state, BurstState::Active);
    }

    #[test]
    fn test_contract_extension() {
        let subtask_id = Uuid::new_v4();
        let contract = BurstContract::new(subtask_id);

        assert!(contract.can_extend());

        let extended = contract.extend(2, Duration::from_secs(30));
        assert!(extended.is_some());

        let ext = extended.unwrap();
        assert_eq!(ext.extensions_granted, 1);
        assert_eq!(ext.max_steps, 2);
    }

    #[test]
    fn test_no_extensions_when_disabled() {
        let subtask_id = Uuid::new_v4();
        let contract = BurstContract::new(subtask_id).without_extensions();

        assert!(!contract.can_extend());
        assert!(contract.extend(2, Duration::from_secs(30)).is_none());
    }

    #[test]
    fn test_context_strategy_overhead() {
        assert!(ContextStrategy::Full.estimated_overhead_tokens() > 1000);
        assert!(ContextStrategy::ArtifactReference.estimated_overhead_tokens() < 200);
    }
}
