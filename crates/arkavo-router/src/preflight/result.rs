//! Moderation result types for pre-flight checks

/// Result of pre-flight moderation check
#[derive(Debug, Clone)]
pub enum ModerationResult {
    /// Request is allowed to proceed
    Allow,
    /// Request is blocked
    Block {
        policy_id: String,
        reason: String,
        feature_values: Vec<(String, bool)>,
    },
}

impl ModerationResult {
    /// Create a block result without feature details
    pub fn block(policy_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Block {
            policy_id: policy_id.into(),
            reason: reason.into(),
            feature_values: Vec::new(),
        }
    }

    /// Create a block result with feature values for debugging
    pub fn block_with_features(
        policy_id: impl Into<String>,
        reason: impl Into<String>,
        features: Vec<(String, bool)>,
    ) -> Self {
        Self::Block {
            policy_id: policy_id.into(),
            reason: reason.into(),
            feature_values: features,
        }
    }

    /// Check if the result allows the request to proceed
    #[must_use]
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }

    /// Check if the result blocks the request
    #[must_use]
    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Block { .. })
    }
}
