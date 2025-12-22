//! Loop detection for preventing strategic thrashing
//!
//! Detects when the orchestrator is repeatedly attempting semantically
//! similar subtasks that keep failing, indicating a strategic dead-end.

use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Maximum number of identical failures before triggering thrashing error
const MAX_IDENTICAL_FAILURES: u32 = 3;

/// Minimum similarity score to consider two descriptions "identical"
const SIMILARITY_THRESHOLD: f64 = 0.85;

/// Detects loops and thrashing in task execution
#[derive(Debug, Default)]
pub struct LoopDetector {
    /// Map of description hashes to failure counts
    failure_counts: HashMap<u64, u32>,
    /// Recent descriptions for similarity comparison
    recent_descriptions: Vec<(u64, String)>,
    /// Maximum descriptions to retain
    max_recent: usize,
}

impl LoopDetector {
    /// Create a new loop detector
    pub fn new() -> Self {
        Self {
            failure_counts: HashMap::new(),
            recent_descriptions: Vec::new(),
            max_recent: 20,
        }
    }

    /// Hash a description for quick comparison
    fn hash_description(description: &str) -> u64 {
        let mut hasher = Sha256::new();
        hasher.update(normalize_description(description).as_bytes());
        let result = hasher.finalize();
        // Take first 8 bytes as u64
        u64::from_le_bytes(result[0..8].try_into().unwrap_or([0u8; 8]))
    }

    /// Record a failed subtask and check for thrashing
    ///
    /// Returns `Some(count)` if thrashing is detected, `None` otherwise.
    pub fn record_failure(&mut self, description: &str) -> Option<u32> {
        let hash = Self::hash_description(description);

        // Check for exact hash match first
        let count = self.failure_counts.entry(hash).or_insert(0);
        *count += 1;

        if *count >= MAX_IDENTICAL_FAILURES {
            return Some(*count);
        }

        // Check for similar descriptions
        for (prev_hash, prev_desc) in &self.recent_descriptions {
            if *prev_hash != hash && similarity(description, prev_desc) >= SIMILARITY_THRESHOLD {
                // Similar but not identical - count as same failure pattern
                let similar_count = self.failure_counts.entry(*prev_hash).or_insert(0);
                *similar_count += 1;

                if *similar_count >= MAX_IDENTICAL_FAILURES {
                    return Some(*similar_count);
                }
            }
        }

        // Store this description
        self.recent_descriptions
            .push((hash, description.to_string()));
        if self.recent_descriptions.len() > self.max_recent {
            self.recent_descriptions.remove(0);
        }

        None
    }

    /// Record a successful subtask (resets failure count for similar tasks)
    pub fn record_success(&mut self, description: &str) {
        let hash = Self::hash_description(description);
        self.failure_counts.remove(&hash);
    }

    /// Check if a description would trigger thrashing detection
    pub fn would_thrash(&self, description: &str) -> bool {
        let hash = Self::hash_description(description);

        if self
            .failure_counts
            .get(&hash)
            .is_some_and(|&count| count >= MAX_IDENTICAL_FAILURES - 1)
        {
            return true;
        }

        // Check similar descriptions
        for (prev_hash, prev_desc) in &self.recent_descriptions {
            if similarity(description, prev_desc) >= SIMILARITY_THRESHOLD
                && self
                    .failure_counts
                    .get(prev_hash)
                    .is_some_and(|&count| count >= MAX_IDENTICAL_FAILURES - 1)
            {
                return true;
            }
        }

        false
    }

    /// Get the failure count for a description
    pub fn failure_count(&self, description: &str) -> u32 {
        let hash = Self::hash_description(description);
        self.failure_counts.get(&hash).copied().unwrap_or(0)
    }

    /// Clear all failure tracking
    pub fn clear(&mut self) {
        self.failure_counts.clear();
        self.recent_descriptions.clear();
    }

    /// Get the hash of a description (for storing in GlobalTaskState)
    pub fn get_hash(description: &str) -> u64 {
        Self::hash_description(description)
    }
}

/// Normalize a description for comparison
fn normalize_description(desc: &str) -> String {
    desc.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Simple similarity score between two descriptions
///
/// Uses word overlap ratio. A more sophisticated implementation
/// could use embedding similarity.
fn similarity(a: &str, b: &str) -> f64 {
    let a_normalized = normalize_description(a);
    let b_normalized = normalize_description(b);

    let a_words: std::collections::HashSet<&str> = a_normalized.split_whitespace().collect();
    let b_words: std::collections::HashSet<&str> = b_normalized.split_whitespace().collect();

    if a_words.is_empty() || b_words.is_empty() {
        return 0.0;
    }

    let intersection = a_words.intersection(&b_words).count();
    let union = a_words.union(&b_words).count();

    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match_detection() {
        let mut detector = LoopDetector::new();

        // First two failures should be fine
        assert!(
            detector
                .record_failure("Fix the authentication bug")
                .is_none()
        );
        assert!(
            detector
                .record_failure("Fix the authentication bug")
                .is_none()
        );

        // Third failure should trigger
        let result = detector.record_failure("Fix the authentication bug");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), 3);
    }

    #[test]
    fn test_similar_description_detection() {
        let mut detector = LoopDetector::new();

        // Similar descriptions should be grouped
        assert!(
            detector
                .record_failure("Fix the authentication bug in login")
                .is_none()
        );
        assert!(
            detector
                .record_failure("Fix authentication bug in the login")
                .is_none()
        );
        assert!(
            detector
                .record_failure("Fix the auth bug in login")
                .is_none()
        );

        // Different description should not trigger immediately
        assert!(
            detector
                .record_failure("Refactor the database layer")
                .is_none()
        );
    }

    #[test]
    fn test_success_resets_count() {
        let mut detector = LoopDetector::new();

        detector.record_failure("Fix the bug");
        detector.record_failure("Fix the bug");

        // Success should reset
        detector.record_success("Fix the bug");

        // Now we can fail again without triggering
        assert!(detector.record_failure("Fix the bug").is_none());
    }

    #[test]
    fn test_would_thrash() {
        let mut detector = LoopDetector::new();

        assert!(!detector.would_thrash("Fix the bug"));

        detector.record_failure("Fix the bug");
        detector.record_failure("Fix the bug");

        // Next failure would cause thrashing
        assert!(detector.would_thrash("Fix the bug"));
    }

    #[test]
    fn test_similarity_function() {
        assert!(similarity("fix the bug", "fix the bug") > 0.99);
        // "fix the authentication bug" vs "fix authentication bug" - Jaccard = 3/4 = 0.75
        assert!(similarity("fix the authentication bug", "fix authentication bug") > 0.7);
        assert!(similarity("fix the bug", "refactor the database") < 0.5);
    }
}
