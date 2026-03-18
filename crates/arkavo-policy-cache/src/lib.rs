//! Runtime PolicyCache with temporal decay for Agent Runtime Policy (§7.2).
//!
//! Stores learned policy entries with configurable decay strategies.
//! Human-sourced entries are exempt from decay per ARP spec §7.2.1.
//! Entries are integrity-protected via hash chaining.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Instant;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use arkavo_arp::feedback::{DecayStrategy, PolicyCacheConfig};

/// Source of a policy entry, used to determine decay exemption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicySource {
    /// Learned from automated feedback — subject to decay.
    Automated,
    /// Taught by a human operator — exempt from decay per §7.2.1.
    Human,
    /// From an incident response — uses quarantine TTL instead.
    Incident,
}

/// A single entry in the PolicyCache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyEntry {
    pub key: String,
    pub value: serde_json::Value,
    pub source: PolicySource,
    pub created_at_epoch_ms: u64,
    /// Hash of the previous entry in the chain (hex string).
    pub prev_hash: String,
    /// Hash of this entry (hex string).
    pub entry_hash: String,
    /// Insertion sequence number for deterministic chain ordering.
    pub seq: u64,
    #[serde(skip)]
    created_at: Option<Instant>,
}

/// Runtime PolicyCache backed by a concurrent map with temporal decay.
pub struct PolicyCache {
    entries: DashMap<String, PolicyEntry>,
    config: PolicyCacheConfig,
    /// Monotonic creation time for decay calculations.
    created_at: Instant,
    /// Hash of the most recently inserted entry (for chain integrity).
    last_hash: std::sync::Mutex<String>,
    /// Monotonically increasing sequence counter.
    next_seq: std::sync::atomic::AtomicU64,
}

impl PolicyCache {
    /// Create a new PolicyCache from ARP config (§7.2.1).
    pub fn new(config: PolicyCacheConfig) -> Self {
        Self {
            entries: DashMap::new(),
            config,
            created_at: Instant::now(),
            last_hash: std::sync::Mutex::new("genesis".to_string()),
            next_seq: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Insert a policy entry. Returns the entry hash for chain verification.
    pub fn insert(&self, key: String, value: serde_json::Value, source: PolicySource) -> String {
        let now = Instant::now();
        let epoch_ms = now.duration_since(self.created_at).as_millis() as u64;

        let seq = self
            .next_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut last = self.last_hash.lock().unwrap();
        let prev = last.clone();
        let entry_hash = compute_hash(&key, &value, &prev, epoch_ms);

        let entry = PolicyEntry {
            key: key.clone(),
            value,
            source,
            created_at_epoch_ms: epoch_ms,
            prev_hash: prev,
            entry_hash: entry_hash.clone(),
            seq,
            created_at: Some(now),
        };

        self.entries.insert(key, entry);
        *last = entry_hash.clone();
        entry_hash
    }

    /// Get an entry by key, applying decay to its influence.
    /// Returns `None` if the entry has expired (TTL exceeded) or doesn't exist.
    pub fn get(&self, key: &str) -> Option<PolicyEntry> {
        let entry = self.entries.get(key)?;
        let entry = entry.value();

        // Human entries exempt from TTL expiration
        if entry.source == PolicySource::Human
            && self.config.human_source_exempt_from_decay.unwrap_or(true)
        {
            return Some(entry.clone());
        }

        let age_secs = entry
            .created_at
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);

        // Check TTL expiration
        if age_secs > self.config.default_ttl_sec as f64 {
            return None;
        }

        Some(entry.clone())
    }

    /// Compute the current influence (0.0–1.0) of an entry based on decay.
    /// Human-sourced entries always return 1.0.
    pub fn influence(&self, key: &str) -> f64 {
        let Some(entry) = self.entries.get(key) else {
            return 0.0;
        };
        let entry = entry.value();

        // Human entries exempt from decay
        if entry.source == PolicySource::Human
            && self.config.human_source_exempt_from_decay.unwrap_or(true)
        {
            return 1.0;
        }

        // Incident entries use quarantine TTL if configured
        if entry.source == PolicySource::Incident
            && let Some(quarantine_sec) = self.config.incident_source_quarantine_sec
        {
            let age = entry.created_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
            if age < quarantine_sec {
                return 1.0;
            }
        }

        let age_secs = entry
            .created_at
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);

        match self.config.decay_strategy {
            DecayStrategy::Exponential => {
                let half_life = self.config.decay_half_life_sec.unwrap_or(86400) as f64;
                0.5_f64.powf(age_secs / half_life)
            }
            DecayStrategy::Linear => {
                let ttl = self.config.default_ttl_sec as f64;
                (1.0 - age_secs / ttl).max(0.0)
            }
            DecayStrategy::Step => {
                let ttl = self.config.default_ttl_sec as f64;
                if age_secs < ttl { 1.0 } else { 0.0 }
            }
            DecayStrategy::None => 1.0,
        }
    }

    /// Remove expired entries (TTL exceeded). Returns count of removed entries.
    pub fn evict_expired(&self) -> usize {
        let ttl_secs = self.config.default_ttl_sec;
        let before = self.entries.len();
        self.entries.retain(|_, entry| {
            // Human entries never expire via TTL
            if entry.source == PolicySource::Human
                && self.config.human_source_exempt_from_decay.unwrap_or(true)
            {
                return true;
            }
            let age = entry.created_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
            age < ttl_secs
        });
        before - self.entries.len()
    }

    /// Verify hash chain integrity. Returns `true` if the chain is valid.
    pub fn verify_chain(&self) -> bool {
        let mut entries: Vec<PolicyEntry> =
            self.entries.iter().map(|e| e.value().clone()).collect();
        entries.sort_by_key(|e| e.seq);

        let mut expected_prev = "genesis".to_string();
        for entry in &entries {
            if entry.prev_hash != expected_prev {
                return false;
            }
            let computed = compute_hash(
                &entry.key,
                &entry.value,
                &entry.prev_hash,
                entry.created_at_epoch_ms,
            );
            if computed != entry.entry_hash {
                return false;
            }
            expected_prev = entry.entry_hash.clone();
        }
        true
    }

    /// Number of entries in the cache.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Compute a deterministic hash for chain integrity.
fn compute_hash(key: &str, value: &serde_json::Value, prev_hash: &str, epoch_ms: u64) -> String {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    value.to_string().hash(&mut hasher);
    prev_hash.hash(&mut hasher);
    epoch_ms.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_config(strategy: DecayStrategy) -> PolicyCacheConfig {
        PolicyCacheConfig {
            default_ttl_sec: 3600,
            decay_strategy: strategy,
            decay_half_life_sec: Some(1),
            human_source_exempt_from_decay: Some(true),
            incident_source_quarantine_sec: Some(7200),
        }
    }

    #[test]
    fn insert_and_get() {
        let cache = PolicyCache::new(test_config(DecayStrategy::None));
        cache.insert(
            "tool.risk.shell".into(),
            json!("high"),
            PolicySource::Automated,
        );
        let entry = cache.get("tool.risk.shell");
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().value, json!("high"));
    }

    #[test]
    fn missing_key_returns_none() {
        let cache = PolicyCache::new(test_config(DecayStrategy::None));
        assert!(cache.get("nonexistent").is_none());
    }

    #[test]
    fn human_entries_exempt_from_decay() {
        let cache = PolicyCache::new(test_config(DecayStrategy::Exponential));
        cache.insert(
            "human.lesson".into(),
            json!("always use model X"),
            PolicySource::Human,
        );
        // Even after some time, human entries have full influence
        let influence = cache.influence("human.lesson");
        assert_eq!(influence, 1.0);
    }

    #[test]
    fn automated_entries_decay_exponentially() {
        // Use a very short half-life to see decay quickly
        let config = PolicyCacheConfig {
            default_ttl_sec: 3600,
            decay_strategy: DecayStrategy::Exponential,
            decay_half_life_sec: Some(1),
            human_source_exempt_from_decay: Some(true),
            incident_source_quarantine_sec: None,
        };
        let cache = PolicyCache::new(config);
        cache.insert("auto.lesson".into(), json!("test"), PolicySource::Automated);

        // Immediately after insertion, influence should be close to 1.0
        let influence = cache.influence("auto.lesson");
        assert!(influence > 0.9, "expected >0.9, got {influence}");

        // Wait for decay
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let influence_after = cache.influence("auto.lesson");
        assert!(
            influence_after < 0.6,
            "expected <0.6 after ~1 half-life, got {influence_after}"
        );
    }

    #[test]
    fn linear_decay() {
        let config = PolicyCacheConfig {
            default_ttl_sec: 2,
            decay_strategy: DecayStrategy::Linear,
            decay_half_life_sec: None,
            human_source_exempt_from_decay: Some(true),
            incident_source_quarantine_sec: None,
        };
        let cache = PolicyCache::new(config);
        cache.insert("linear".into(), json!("test"), PolicySource::Automated);

        let initial = cache.influence("linear");
        assert!(initial > 0.9);

        std::thread::sleep(std::time::Duration::from_millis(1100));
        let mid = cache.influence("linear");
        assert!(mid < 0.6, "expected <0.6 at halfway, got {mid}");
    }

    #[test]
    fn step_decay() {
        let config = PolicyCacheConfig {
            default_ttl_sec: 1,
            decay_strategy: DecayStrategy::Step,
            decay_half_life_sec: None,
            human_source_exempt_from_decay: Some(true),
            incident_source_quarantine_sec: None,
        };
        let cache = PolicyCache::new(config);
        cache.insert("step".into(), json!("test"), PolicySource::Automated);

        assert_eq!(cache.influence("step"), 1.0);

        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert_eq!(cache.influence("step"), 0.0);
    }

    #[test]
    fn hash_chain_integrity() {
        let cache = PolicyCache::new(test_config(DecayStrategy::None));
        cache.insert("a".into(), json!(1), PolicySource::Automated);
        cache.insert("b".into(), json!(2), PolicySource::Human);
        cache.insert("c".into(), json!(3), PolicySource::Automated);
        assert!(cache.verify_chain());
    }

    #[test]
    fn evict_expired_entries() {
        let config = PolicyCacheConfig {
            default_ttl_sec: 1,
            decay_strategy: DecayStrategy::None,
            decay_half_life_sec: None,
            human_source_exempt_from_decay: Some(true),
            incident_source_quarantine_sec: None,
        };
        let cache = PolicyCache::new(config);
        cache.insert("ephemeral".into(), json!("temp"), PolicySource::Automated);
        cache.insert("permanent".into(), json!("forever"), PolicySource::Human);

        assert_eq!(cache.len(), 2);

        std::thread::sleep(std::time::Duration::from_millis(1100));
        let evicted = cache.evict_expired();
        assert_eq!(evicted, 1);
        assert_eq!(cache.len(), 1);
        assert!(cache.get("permanent").is_some());
    }

    #[test]
    fn influence_of_missing_key_is_zero() {
        let cache = PolicyCache::new(test_config(DecayStrategy::Exponential));
        assert_eq!(cache.influence("missing"), 0.0);
    }

    #[test]
    fn empty_cache() {
        let cache = PolicyCache::new(test_config(DecayStrategy::None));
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert!(cache.verify_chain());
    }
}
