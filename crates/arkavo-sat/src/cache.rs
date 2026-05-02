//! LRU cache for boundary probe results
//!
//! Caches probe results to avoid redundant computation, with TTL and
//! graph-hash invalidation support.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lru::LruCache;
use torg_core::Graph;

use crate::boundary::BoundaryProbe;
use crate::error::SatResult;

/// Default cache capacity
pub const DEFAULT_CACHE_CAPACITY: usize = 1000;

/// Default TTL for cache entries
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_mins(5);

/// Cache key uniquely identifying a probe query
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ProbeCacheKey {
    /// Hash of the graph structure
    pub graph_hash: u64,
    /// Output node ID being probed
    pub output_id: u16,
    /// Epsilon distance for boundary probing
    pub epsilon: u32,
}

impl ProbeCacheKey {
    /// Create a new cache key
    pub fn new(graph: &Graph, output_id: u16, epsilon: u32) -> Self {
        Self {
            graph_hash: compute_graph_hash(graph),
            output_id,
            epsilon,
        }
    }
}

/// Cached probe result with expiration
#[derive(Debug, Clone)]
struct CacheEntry {
    probes: Vec<BoundaryProbe>,
    created_at: Instant,
}

/// LRU cache for boundary probe results
///
/// Thread-safe with mutex protection for concurrent access.
pub struct ProbeCache {
    cache: Arc<Mutex<LruCache<ProbeCacheKey, CacheEntry>>>,
    ttl: Duration,
}

impl ProbeCache {
    /// Create a new probe cache with default settings
    pub fn new() -> Self {
        Self::with_config(DEFAULT_CACHE_CAPACITY, DEFAULT_CACHE_TTL)
    }

    /// Create a new probe cache with custom capacity and TTL
    pub fn with_config(capacity: usize, ttl: Duration) -> Self {
        let cap = NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::new(1).unwrap());
        Self {
            cache: Arc::new(Mutex::new(LruCache::new(cap))),
            ttl,
        }
    }

    /// Get cached probes for a key
    ///
    /// Returns None if not found or expired.
    pub fn get(&self, key: &ProbeCacheKey) -> Option<Vec<BoundaryProbe>> {
        let mut cache = self.cache.lock().ok()?;

        if let Some(entry) = cache.get(key) {
            // Check TTL
            if entry.created_at.elapsed() <= self.ttl {
                return Some(entry.probes.clone());
            }
            // Expired - remove it
            cache.pop(key);
        }
        None
    }

    /// Store probes in the cache
    pub fn put(&self, key: ProbeCacheKey, probes: Vec<BoundaryProbe>) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.put(
                key,
                CacheEntry {
                    probes,
                    created_at: Instant::now(),
                },
            );
        }
    }

    /// Invalidate all entries for a specific graph
    pub fn invalidate_graph(&self, graph_hash: u64) {
        if let Ok(mut cache) = self.cache.lock() {
            // Collect keys to remove (can't modify while iterating)
            let keys_to_remove: Vec<ProbeCacheKey> = cache
                .iter()
                .filter(|(k, _)| k.graph_hash == graph_hash)
                .map(|(k, _)| k.clone())
                .collect();

            for key in keys_to_remove {
                cache.pop(&key);
            }
        }
    }

    /// Invalidate all cache entries
    pub fn invalidate_all(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.clear();
        }
    }

    /// Get current cache size
    pub fn len(&self) -> usize {
        self.cache.lock().map(|c| c.len()).unwrap_or(0)
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get cache hit rate statistics
    pub fn stats(&self) -> CacheStats {
        // Note: LRU crate doesn't track hits/misses, so we'd need to add that
        CacheStats {
            size: self.len(),
            capacity: self
                .cache
                .lock()
                .map(|c| c.cap().get())
                .unwrap_or(DEFAULT_CACHE_CAPACITY),
        }
    }
}

impl Default for ProbeCache {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ProbeCache {
    fn clone(&self) -> Self {
        // Create a new cache with same config but empty contents
        Self {
            cache: Arc::clone(&self.cache),
            ttl: self.ttl,
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// Current number of entries
    pub size: usize,
    /// Maximum capacity
    pub capacity: usize,
}

/// Compute a stable hash of a graph structure
///
/// The hash includes all nodes, their operators, and connections,
/// so any structural change will produce a different hash.
pub fn compute_graph_hash(graph: &Graph) -> u64 {
    let mut hasher = DefaultHasher::new();

    // Hash inputs
    graph.inputs.hash(&mut hasher);

    // Hash outputs
    graph.outputs.hash(&mut hasher);

    // Hash nodes (order matters for structural identity)
    for node in &graph.nodes {
        node.id.hash(&mut hasher);
        // Hash the operator
        match node.op {
            torg_core::token::BoolOp::Or => 0u8.hash(&mut hasher),
            torg_core::token::BoolOp::Nor => 1u8.hash(&mut hasher),
            torg_core::token::BoolOp::Xor => 2u8.hash(&mut hasher),
        }
        // Hash sources
        hash_source(&node.left, &mut hasher);
        hash_source(&node.right, &mut hasher);
    }

    hasher.finish()
}

fn hash_source(source: &torg_core::token::Source, hasher: &mut impl Hasher) {
    match source {
        torg_core::token::Source::Id(id) => {
            0u8.hash(hasher);
            id.hash(hasher);
        }
        torg_core::token::Source::True => 1u8.hash(hasher),
        torg_core::token::Source::False => 2u8.hash(hasher),
    }
}

/// Cached boundary probing function
///
/// Uses the cache to avoid redundant computation.
pub fn find_epsilon_boundary_cached(
    graph: &Graph,
    output_id: u16,
    epsilon: u32,
    cache: &ProbeCache,
) -> SatResult<Vec<BoundaryProbe>> {
    let key = ProbeCacheKey::new(graph, output_id, epsilon);

    // Check cache first
    if let Some(probes) = cache.get(&key) {
        return Ok(probes);
    }

    // Compute and cache
    let probes = crate::boundary::find_epsilon_boundary(graph, output_id, epsilon)?;
    cache.put(key, probes.clone());
    Ok(probes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use torg_core::{Builder, Token};

    fn create_or_graph() -> Graph {
        let mut builder = Builder::new();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.push(Token::Or).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::NodeEnd).unwrap();
        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.finish().unwrap()
    }

    fn create_and_graph() -> Graph {
        // AND = NOR(NOR(a,a), NOR(b,b))
        let mut builder = Builder::new();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.push(Token::Nor).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::NodeEnd).unwrap();
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(3)).unwrap();
        builder.push(Token::Nor).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::NodeEnd).unwrap();
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(4)).unwrap();
        builder.push(Token::Nor).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.push(Token::Id(3)).unwrap();
        builder.push(Token::NodeEnd).unwrap();
        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(4)).unwrap();
        builder.finish().unwrap()
    }

    #[test]
    fn test_cache_put_get() {
        let cache = ProbeCache::new();
        let graph = create_or_graph();
        let key = ProbeCacheKey::new(&graph, 2, 1);

        // Initially empty
        assert!(cache.get(&key).is_none());
        assert!(cache.is_empty());

        // Put and get
        let probes = vec![BoundaryProbe {
            input_assignment: [(0, true), (1, false)].into_iter().collect(),
            output_id: 2,
            output_value: true,
            distance_to_flip: 1,
        }];
        cache.put(key.clone(), probes.clone());

        assert_eq!(cache.len(), 1);
        let cached = cache.get(&key);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().len(), 1);
    }

    #[test]
    fn test_cache_ttl_expiration() {
        // Short TTL for testing
        let cache = ProbeCache::with_config(10, Duration::from_millis(50));
        let graph = create_or_graph();
        let key = ProbeCacheKey::new(&graph, 2, 1);

        let probes = vec![BoundaryProbe {
            input_assignment: [(0, true)].into_iter().collect(),
            output_id: 2,
            output_value: true,
            distance_to_flip: 1,
        }];
        cache.put(key.clone(), probes);

        // Should be present immediately
        assert!(cache.get(&key).is_some());

        // Wait for expiration
        sleep(Duration::from_millis(60));

        // Should be expired
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn test_cache_invalidate_graph() {
        let cache = ProbeCache::new();
        let graph1 = create_or_graph();
        let graph2 = create_and_graph();

        let key1 = ProbeCacheKey::new(&graph1, 2, 1);
        let key2 = ProbeCacheKey::new(&graph2, 4, 1);

        let probes = vec![];
        cache.put(key1.clone(), probes.clone());
        cache.put(key2.clone(), probes);

        assert_eq!(cache.len(), 2);

        // Invalidate graph1
        cache.invalidate_graph(key1.graph_hash);

        assert_eq!(cache.len(), 1);
        assert!(cache.get(&key1).is_none());
        assert!(cache.get(&key2).is_some());
    }

    #[test]
    fn test_cache_invalidate_all() {
        let cache = ProbeCache::new();
        let graph = create_or_graph();

        let key1 = ProbeCacheKey::new(&graph, 2, 1);
        let key2 = ProbeCacheKey::new(&graph, 2, 2);

        cache.put(key1, vec![]);
        cache.put(key2, vec![]);

        assert_eq!(cache.len(), 2);

        cache.invalidate_all();

        assert!(cache.is_empty());
    }

    #[test]
    fn test_graph_hash_stability() {
        let graph = create_or_graph();

        let hash1 = compute_graph_hash(&graph);
        let hash2 = compute_graph_hash(&graph);

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_graph_hash_different_graphs() {
        let or_graph = create_or_graph();
        let and_graph = create_and_graph();

        let hash1 = compute_graph_hash(&or_graph);
        let hash2 = compute_graph_hash(&and_graph);

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_cached_boundary_probing() {
        let cache = ProbeCache::new();
        let graph = create_or_graph();

        // First call should compute
        let probes1 = find_epsilon_boundary_cached(&graph, 2, 1, &cache).unwrap();

        // Second call should use cache
        let probes2 = find_epsilon_boundary_cached(&graph, 2, 1, &cache).unwrap();

        assert_eq!(probes1.len(), probes2.len());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_lru_eviction() {
        // Small capacity
        let cache = ProbeCache::with_config(2, DEFAULT_CACHE_TTL);
        let graph = create_or_graph();

        let key1 = ProbeCacheKey::new(&graph, 2, 1);
        let key2 = ProbeCacheKey::new(&graph, 2, 2);
        let key3 = ProbeCacheKey::new(&graph, 2, 3);

        cache.put(key1.clone(), vec![]);
        cache.put(key2.clone(), vec![]);
        assert_eq!(cache.len(), 2);

        // Adding third should evict first (LRU)
        cache.put(key3, vec![]);
        assert_eq!(cache.len(), 2);
        assert!(cache.get(&key1).is_none()); // Evicted
        assert!(cache.get(&key2).is_some());
    }
}
