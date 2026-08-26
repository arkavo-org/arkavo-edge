use crate::types::Decision;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct CacheKey {
    pdp_origin: String,
    subject_id: String,
    action_name: String,
    resource_type: String,
    resource_id: String,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    decision: Decision,
    expires_at: Instant,
}

pub struct DecisionCache {
    cache: Arc<Mutex<LruCache<CacheKey, CacheEntry>>>,
    default_ttl: Duration,
}

impl DecisionCache {
    /// Creates a new decision cache with the specified capacity and default TTL.
    ///
    /// # Panics
    ///
    /// Panics only if `NonZeroUsize::new(1000)` fails, which cannot happen.
    pub fn new(capacity: usize, default_ttl: Duration) -> Self {
        let capacity = NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::new(1000).unwrap());
        Self {
            cache: Arc::new(Mutex::new(LruCache::new(capacity))),
            default_ttl,
        }
    }

    /// # Panics
    ///
    /// Panics if the cache mutex is poisoned.
    pub fn get(
        &self,
        pdp_origin: &str,
        subject_id: &str,
        action_name: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> Option<Decision> {
        let key = Self::make_key(
            pdp_origin,
            subject_id,
            action_name,
            resource_type,
            resource_id,
        );

        let mut cache = self.cache.lock().unwrap();
        if let Some(entry) = cache.get(&key) {
            if entry.expires_at > Instant::now() {
                return Some(entry.decision.clone());
            }
            cache.pop(&key);
        }
        None
    }

    /// # Panics
    ///
    /// Panics if the cache mutex is poisoned.
    #[allow(clippy::too_many_arguments)]
    pub fn put(
        &self,
        pdp_origin: &str,
        subject_id: &str,
        action_name: &str,
        resource_type: &str,
        resource_id: &str,
        decision: Decision,
        ttl: Option<Duration>,
    ) {
        let key = Self::make_key(
            pdp_origin,
            subject_id,
            action_name,
            resource_type,
            resource_id,
        );
        let ttl = ttl.unwrap_or(self.default_ttl);
        let entry = CacheEntry {
            decision,
            expires_at: Instant::now() + ttl,
        };

        let mut cache = self.cache.lock().unwrap();
        cache.put(key, entry);
    }

    /// # Panics
    ///
    /// Panics if the cache mutex is poisoned.
    pub fn clear(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.clear();
    }

    /// # Panics
    ///
    /// Panics if the cache mutex is poisoned.
    pub fn evict_expired(&self) {
        let now = Instant::now();
        let mut cache = self.cache.lock().unwrap();

        let expired_keys: Vec<CacheKey> = cache
            .iter()
            .filter(|(_, entry)| entry.expires_at <= now)
            .map(|(key, _)| key.clone())
            .collect();

        for key in expired_keys {
            cache.pop(&key);
        }
    }

    fn make_key(
        pdp_origin: &str,
        subject_id: &str,
        action_name: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> CacheKey {
        CacheKey {
            pdp_origin: pdp_origin.to_string(),
            subject_id: subject_id.to_string(),
            action_name: action_name.to_string(),
            resource_type: resource_type.to_string(),
            resource_id: resource_id.to_string(),
        }
    }

    pub fn calculate_ttl_from_token(token_exp: Option<i64>) -> Duration {
        const MAX_TTL: Duration = Duration::from_mins(1);

        if let Some(exp) = token_exp {
            let now = chrono::Utc::now().timestamp();
            let remaining = exp - now;

            if remaining > 0 {
                let ttl = Duration::from_secs(remaining as u64);
                return ttl.min(MAX_TTL);
            }
        }

        MAX_TTL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_basic_operations() {
        let cache = DecisionCache::new(100, Duration::from_mins(1));
        let pdp = "https://kas.arkavo.net";

        assert!(
            cache
                .get(pdp, "user123", "tools/call", "tool", "git_commit")
                .is_none()
        );

        cache.put(
            pdp,
            "user123",
            "tools/call",
            "tool",
            "git_commit",
            Decision::Permit,
            None,
        );
        assert_eq!(
            cache.get(pdp, "user123", "tools/call", "tool", "git_commit"),
            Some(Decision::Permit)
        );

        cache.clear();
        assert!(
            cache
                .get(pdp, "user123", "tools/call", "tool", "git_commit")
                .is_none()
        );
    }

    #[test]
    fn test_cache_expiration() {
        let cache = DecisionCache::new(100, Duration::from_mins(1));
        let pdp = "https://kas.arkavo.net";

        cache.put(
            pdp,
            "user123",
            "tools/call",
            "tool",
            "git_commit",
            Decision::Permit,
            Some(Duration::from_millis(1)),
        );
        std::thread::sleep(Duration::from_millis(2));
        assert!(
            cache
                .get(pdp, "user123", "tools/call", "tool", "git_commit")
                .is_none()
        );
    }

    #[test]
    fn test_cache_key_includes_pdp_origin() {
        let cache = DecisionCache::new(100, Duration::from_mins(1));

        cache.put(
            "https://pdp-a",
            "user123",
            "tools/call",
            "tool",
            "git_commit",
            Decision::Permit,
            None,
        );
        cache.put(
            "https://pdp-b",
            "user123",
            "tools/call",
            "tool",
            "git_commit",
            Decision::Deny,
            None,
        );

        assert_eq!(
            cache.get(
                "https://pdp-a",
                "user123",
                "tools/call",
                "tool",
                "git_commit"
            ),
            Some(Decision::Permit)
        );
        assert_eq!(
            cache.get(
                "https://pdp-b",
                "user123",
                "tools/call",
                "tool",
                "git_commit"
            ),
            Some(Decision::Deny)
        );
    }
}
