use crate::types::{Action, Decision, EntityIdentifier, Resource};
use lru::LruCache;
use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct CacheKey {
    entity_id: String,
    action_name: String,
    resource_fqns: BTreeSet<String>,
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
    /// This function will panic if unable to create a NonZeroUsize with value 1000,
    /// which should never happen in practice.
    pub fn new(capacity: usize, default_ttl: Duration) -> Self {
        let capacity = NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::new(1000).unwrap());
        Self {
            cache: Arc::new(Mutex::new(LruCache::new(capacity))),
            default_ttl,
        }
    }

    /// Gets a cached decision for the given entity, action, and resource.
    ///
    /// # Panics
    ///
    /// This function will panic if the cache mutex is poisoned.
    pub fn get(
        &self,
        entity: &EntityIdentifier,
        action: &Action,
        resource: &Resource,
    ) -> Option<Decision> {
        let key = Self::make_key(entity, action, resource);

        let mut cache = self.cache.lock().unwrap();
        if let Some(entry) = cache.get(&key) {
            if entry.expires_at > Instant::now() {
                return Some(entry.decision.clone());
            }
            cache.pop(&key);
        }
        None
    }

    /// Puts a decision into the cache for the given entity, action, and resource.
    ///
    /// # Panics
    ///
    /// This function will panic if the cache mutex is poisoned.
    pub fn put(
        &self,
        entity: &EntityIdentifier,
        action: &Action,
        resource: &Resource,
        decision: Decision,
        ttl: Option<Duration>,
    ) {
        let key = Self::make_key(entity, action, resource);
        let ttl = ttl.unwrap_or(self.default_ttl);
        let entry = CacheEntry {
            decision,
            expires_at: Instant::now() + ttl,
        };

        let mut cache = self.cache.lock().unwrap();
        cache.put(key, entry);
    }

    /// Clears all entries from the cache.
    ///
    /// # Panics
    ///
    /// This function will panic if the cache mutex is poisoned.
    pub fn clear(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.clear();
    }

    /// Evicts expired entries from the cache.
    ///
    /// # Panics
    ///
    /// This function will panic if the cache mutex is poisoned.
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

    fn make_key(entity: &EntityIdentifier, action: &Action, resource: &Resource) -> CacheKey {
        let resource_fqns = resource
            .attribute_value_fqns
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();

        CacheKey {
            entity_id: entity.id.clone(),
            action_name: action.name.clone(),
            resource_fqns,
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

        let entity = EntityIdentifier {
            id: "user123".to_string(),
            entity_type: crate::types::EntityType::Subject,
            attributes: None,
        };

        let action = Action::execute_tool();
        let resource = Resource::mcp_tool("git.commit");

        // Cache miss
        assert!(cache.get(&entity, &action, &resource).is_none());

        // Put and get
        cache.put(&entity, &action, &resource, Decision::Permit, None);
        assert_eq!(
            cache.get(&entity, &action, &resource),
            Some(Decision::Permit)
        );

        // Clear
        cache.clear();
        assert!(cache.get(&entity, &action, &resource).is_none());
    }

    #[test]
    fn test_cache_expiration() {
        let cache = DecisionCache::new(100, Duration::from_mins(1));

        let entity = EntityIdentifier {
            id: "user123".to_string(),
            entity_type: crate::types::EntityType::Subject,
            attributes: None,
        };

        let action = Action::execute_tool();
        let resource = Resource::mcp_tool("git.commit");

        // Put with very short TTL
        cache.put(
            &entity,
            &action,
            &resource,
            Decision::Permit,
            Some(Duration::from_millis(1)),
        );

        // Wait for expiration
        std::thread::sleep(Duration::from_millis(2));

        // Should be expired
        assert!(cache.get(&entity, &action, &resource).is_none());
    }

    #[test]
    fn test_cache_key_uniqueness() {
        let cache = DecisionCache::new(100, Duration::from_mins(1));

        let entity1 = EntityIdentifier {
            id: "user123".to_string(),
            entity_type: crate::types::EntityType::Subject,
            attributes: None,
        };

        let entity2 = EntityIdentifier {
            id: "user456".to_string(),
            entity_type: crate::types::EntityType::Subject,
            attributes: None,
        };

        let action = Action::execute_tool();
        let resource = Resource::mcp_tool("git.commit");

        // Different entities should have different cache entries
        cache.put(&entity1, &action, &resource, Decision::Permit, None);
        cache.put(&entity2, &action, &resource, Decision::Deny, None);

        assert_eq!(
            cache.get(&entity1, &action, &resource),
            Some(Decision::Permit)
        );
        assert_eq!(
            cache.get(&entity2, &action, &resource),
            Some(Decision::Deny)
        );
    }
}
