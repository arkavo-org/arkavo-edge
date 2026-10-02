//! Context Pool - bounded, reusable inference contexts per model
//!
//! Each context owns a KV cache, which is the largest private allocation an
//! agent process makes, so the number of contexts per model is a hard bound:
//! - contexts are created lazily, up to the bound, and reused afterwards
//! - a caller that finds every context busy waits for one to be released
//! - nothing outside the pool creates a context for a pooled model

#[cfg(feature = "llama-cpp")]
mod slots;

use std::collections::HashMap;
#[cfg(not(feature = "llama-cpp"))]
use std::collections::HashSet;
#[cfg(feature = "llama-cpp")]
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

use crate::{Error, Result};

/// Contexts per model unless `ARKAVO_MAX_CONTEXTS` says otherwise.
///
/// One, because a swarm runs each role as its own process and every extra
/// context costs another KV cache. The router admits at most one request per
/// purpose (task, chat, synthesis), so at most two callers queue behind the
/// one that is generating, and a context is only held while tokens are
/// produced, never across a call that needs another context.
pub const DEFAULT_MAX_CONTEXTS: usize = 1;

/// Wait for a busy context unless `ARKAVO_CONTEXT_WAIT_SECS` says otherwise.
///
/// Long enough for the two generations that can be queued ahead of a
/// caller, short enough that a stuck context surfaces as an error instead
/// of a hung agent.
pub const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_mins(5);

/// Contexts each model may have in this process.
pub fn default_max_contexts() -> usize {
    parse_positive(std::env::var("ARKAVO_MAX_CONTEXTS").ok().as_deref())
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(DEFAULT_MAX_CONTEXTS)
}

/// How long a caller waits for a busy context in this process.
pub fn default_acquire_timeout() -> Duration {
    parse_positive(std::env::var("ARKAVO_CONTEXT_WAIT_SECS").ok().as_deref())
        .map_or(DEFAULT_ACQUIRE_TIMEOUT, Duration::from_secs)
}

/// Zero is rejected: no contexts, or no time to wait for one, would fail
/// every request.
fn parse_positive(raw: Option<&str>) -> Option<u64> {
    raw.and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&n| n > 0)
}

/// Statistics for a model's context pool
#[derive(Debug, Clone, Copy)]
pub struct PoolStats {
    pub available: usize,
    pub in_use: usize,
    pub max: usize,
}

impl PoolStats {
    pub fn total(&self) -> usize {
        self.available + self.in_use
    }

    pub fn utilization_pct(&self) -> f64 {
        if self.max == 0 {
            0.0
        } else {
            (self.in_use as f64 / self.max as f64) * 100.0
        }
    }
}

/// Stub implementation for non-llama-cpp builds
#[cfg(not(feature = "llama-cpp"))]
pub struct ContextPool {
    _pools: RwLock<HashSet<String>>,
    _default_max_contexts: usize,
}

#[cfg(not(feature = "llama-cpp"))]
impl ContextPool {
    pub fn new() -> Self {
        Self::with_max_contexts(default_max_contexts())
    }

    pub fn with_max_contexts(max_contexts: usize) -> Self {
        Self {
            _pools: RwLock::new(HashSet::new()),
            _default_max_contexts: max_contexts,
        }
    }

    pub fn register_model(&self, _name: &str, _model: ()) -> Result<()> {
        Err(Error::Config(
            "llama-cpp feature not enabled - rebuild with --features llama-cpp".to_string(),
        ))
    }

    pub fn acquire(&self, model_name: &str) -> Result<()> {
        Err(Error::Config(format!(
            "Model '{model_name}' not available (llama-cpp not enabled)"
        )))
    }

    pub fn release(&self, _model_name: &str, _context: ()) -> Result<()> {
        Ok(())
    }

    pub fn stats(&self, _model_name: &str) -> Option<PoolStats> {
        None
    }

    pub fn all_stats(&self) -> HashMap<String, PoolStats> {
        HashMap::new()
    }
}

#[cfg(not(feature = "llama-cpp"))]
impl Default for ContextPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "llama-cpp")]
use arkavo_llama_cpp::{LlamaContext, LlamaModel};
#[cfg(feature = "llama-cpp")]
use slots::{Claim, Slots};
#[cfg(feature = "llama-cpp")]
use std::sync::Mutex;

/// A pooled context with its associated model
#[cfg(feature = "llama-cpp")]
pub struct PooledContext {
    pub context: Arc<Mutex<LlamaContext>>,
    pub model_name: String,
    pub created_at: std::time::Instant,
    pub use_count: usize,
    /// Current token position in KV cache (for resuming generation)
    pub token_position: i32,
    /// Optional context manager for multi-sequence KV cache slots
    #[cfg(feature = "llama-cpp")]
    pub context_manager: Option<arkavo_kv_cache::ContextManager>,
}

#[cfg(feature = "llama-cpp")]
impl PooledContext {
    fn new(context: LlamaContext, model_name: String) -> Self {
        Self {
            context: Arc::new(Mutex::new(context)),
            model_name,
            created_at: std::time::Instant::now(),
            use_count: 0,
            token_position: 0,
            #[cfg(feature = "llama-cpp")]
            context_manager: None,
        }
    }

    /// Clear the KV cache to prepare for a new conversation
    pub fn clear_kv_cache(&self) {
        if let Ok(ctx) = self.context.lock() {
            ctx.clear_kv_cache();
        }
        // Note: caller should reset token_position after this
    }

    /// Get the current token position
    pub fn get_token_position(&self) -> i32 {
        self.token_position
    }

    /// Set the token position (after generation)
    pub fn set_token_position(&mut self, pos: i32) {
        self.token_position = pos;
    }

    fn mark_used(&mut self) {
        self.use_count += 1;
    }

    fn reset(&mut self) {
        self.clear_kv_cache();
        self.token_position = 0;
    }
}

/// Pool of contexts for a specific model
#[cfg(feature = "llama-cpp")]
struct ModelContextPool {
    // Declared before `model` so idle contexts are freed before the model
    // they were created from.
    slots: Slots<PooledContext>,
    model: Arc<LlamaModel>,
}

#[cfg(feature = "llama-cpp")]
impl ModelContextPool {
    fn new(model: Arc<LlamaModel>, max_contexts: usize) -> Self {
        Self {
            slots: Slots::new(max_contexts),
            model,
        }
    }

    /// Turn a claimed slot into a context, creating one if the slot is new.
    fn fill(&self, claim: Claim<PooledContext>, clear_cache: bool) -> Result<PooledContext> {
        match claim {
            Claim::Idle(mut context) => {
                if clear_cache {
                    context.reset();
                }
                context.mark_used();
                Ok(context)
            }
            Claim::Vacant => self.create(LlamaContext::new(&self.model)),
            Claim::Exhausted => Err(self.exhausted()),
        }
    }

    /// Wrap a newly created context, or free the slot reserved for it.
    fn create(&self, created: std::result::Result<LlamaContext, String>) -> Result<PooledContext> {
        match created {
            Ok(context) => {
                let mut pooled = PooledContext::new(context, self.model_name());
                pooled.mark_used();
                Ok(pooled)
            }
            Err(e) => {
                self.slots.forfeit();
                Err(Error::Config(format!("Failed to create context: {e}")))
            }
        }
    }

    fn exhausted(&self) -> Error {
        Error::Internal(format!(
            "Max contexts ({}) reached for model. All contexts in use.",
            self.slots.max()
        ))
    }

    /// Acquire a context with multi-sequence support (learning + conversation).
    /// Creates a context via `new_with_sequences(model, 2, true)` and attaches
    /// a `ContextManager` with seq_learning=0, seq_conversation=1.
    fn acquire_multi_seq(&self) -> Result<PooledContext> {
        if !self.slots.claim_vacant() {
            return Err(self.exhausted());
        }
        let mut pooled = self.create(LlamaContext::new_with_sequences(&self.model, 2, true))?;
        #[cfg(feature = "llama-cpp")]
        {
            pooled.context_manager = Some(arkavo_kv_cache::ContextManager::new(0, 1));
        }
        Ok(pooled)
    }

    /// Release a context back to the pool
    fn release(&self, mut context: PooledContext, clear_cache: bool) {
        // A poisoned lock means generation panicked while holding the
        // context; its llama.cpp state is unknown, so the slot is freed and
        // the next caller gets a new context.
        if context.context.is_poisoned() {
            tracing::warn!(
                model = %context.model_name,
                "Discarding a context whose last user panicked"
            );
            self.slots.forfeit();
            return;
        }
        if clear_cache {
            context.reset();
        }
        self.slots.give_back(context);
    }

    fn model_name(&self) -> String {
        self.model.model_name().to_string()
    }

    fn stats(&self) -> PoolStats {
        let (available, in_use) = self.slots.counts();
        PoolStats {
            available,
            in_use,
            max: self.slots.max(),
        }
    }
}

/// Manages pools of contexts for multiple models
#[cfg(feature = "llama-cpp")]
pub struct ContextPool {
    pools: RwLock<HashMap<String, Arc<ModelContextPool>>>,
    default_max_contexts: usize,
}

#[cfg(feature = "llama-cpp")]
impl ContextPool {
    pub fn new() -> Self {
        Self::with_max_contexts(default_max_contexts())
    }

    pub fn with_max_contexts(max_contexts: usize) -> Self {
        Self {
            pools: RwLock::new(HashMap::new()),
            default_max_contexts: max_contexts,
        }
    }

    /// Contexts each registered model may have.
    pub const fn max_contexts(&self) -> usize {
        self.default_max_contexts
    }

    #[allow(clippy::significant_drop_tightening)]
    pub fn register_model(&self, name: &str, model: Arc<LlamaModel>) -> Result<()> {
        self.pools
            .write()
            .map_err(|_| Error::Internal("Pool lock poisoned".to_string()))?
            .insert(
                name.to_string(),
                Arc::new(ModelContextPool::new(model, self.default_max_contexts)),
            );
        Ok(())
    }

    /// The model's pool, cloned out so the map lock is not held while a
    /// context is created or waited for.
    fn pool_for(&self, model_name: &str) -> Result<Arc<ModelContextPool>> {
        self.pools
            .read()
            .map_err(|_| Error::Internal("Pool lock poisoned".to_string()))?
            .get(model_name)
            .cloned()
            .ok_or_else(|| Error::Config(format!("Model '{model_name}' not registered in pool")))
    }

    /// Acquire a context preserving KV cache (for multi-turn conversations).
    /// Fails at once when every context is in use.
    pub fn acquire(&self, model_name: &str) -> Result<PooledContext> {
        let pool = self.pool_for(model_name)?;
        pool.fill(pool.slots.claim(), false)
    }

    /// Acquire a fresh context with cleared KV cache (for new conversations).
    /// Fails at once when every context is in use.
    pub fn acquire_fresh(&self, model_name: &str) -> Result<PooledContext> {
        let pool = self.pool_for(model_name)?;
        pool.fill(pool.slots.claim(), true)
    }

    /// Acquire a fresh context, waiting up to `limit` for one to be released
    /// when every context is in use.
    pub async fn acquire_fresh_within(
        &self,
        model_name: &str,
        limit: Duration,
    ) -> Result<PooledContext> {
        let pool = self.pool_for(model_name)?;
        let claim = pool.slots.claim_within(limit).await.ok_or_else(|| {
            Error::Inference(format!(
                "Timed out after {}s waiting for a free inference context for model                  '{model_name}' ({} allowed, all in use)",
                limit.as_secs(),
                pool.slots.max()
            ))
        })?;
        pool.fill(claim, true)
    }

    /// Acquire a context with multi-sequence support for KV cache context slots.
    /// The returned `PooledContext` has a `ContextManager` attached.
    pub fn acquire_multi_seq(&self, model_name: &str) -> Result<PooledContext> {
        self.pool_for(model_name)?.acquire_multi_seq()
    }

    /// Release a context back to the pool
    ///
    /// # Arguments
    /// * `model_name` - Name of the model this context belongs to
    /// * `context` - The context to release
    /// * `clear_cache` - If true, clears KV cache before returning to pool
    pub fn release(
        &self,
        model_name: &str,
        context: PooledContext,
        clear_cache: bool,
    ) -> Result<()> {
        let pool = self
            .pools
            .read()
            .map_err(|_| Error::Internal("Pool lock poisoned".to_string()))?
            .get(model_name)
            .cloned()
            .ok_or_else(|| Error::Config(format!("Model '{model_name}' not found")))?;
        pool.release(context, clear_cache);
        Ok(())
    }

    pub fn stats(&self, model_name: &str) -> Option<PoolStats> {
        self.pools
            .read()
            .ok()
            .and_then(|pools| pools.get(model_name).map(|p| p.stats()))
    }

    pub fn all_stats(&self) -> HashMap<String, PoolStats> {
        self.pools
            .read()
            .ok()
            .map(|pools| {
                pools
                    .iter()
                    .map(|(name, pool)| (name.clone(), pool.stats()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(feature = "llama-cpp")]
impl Default for ContextPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_creation() {
        let pool = ContextPool::new();
        let stats = pool.all_stats();
        assert!(stats.is_empty());
    }

    #[test]
    fn test_pool_default() {
        let pool = ContextPool::default();
        let stats = pool.all_stats();
        assert!(stats.is_empty());
    }

    #[test]
    fn test_pool_stats_empty() {
        let pool = ContextPool::new();
        assert!(pool.stats("any-model").is_none());
    }

    #[test]
    fn test_pool_thread_safety() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ContextPool>();
    }

    /// Regression: the default used to be four contexts per model, each
    /// with its own KV cache.
    #[test]
    fn one_context_per_model_by_default() {
        assert_eq!(DEFAULT_MAX_CONTEXTS, 1);
    }

    #[test]
    fn overrides_must_be_positive_numbers() {
        assert_eq!(parse_positive(Some("2")), Some(2));
        assert_eq!(parse_positive(Some(" 30 ")), Some(30));
        assert_eq!(parse_positive(Some("0")), None);
        assert_eq!(parse_positive(Some("-1")), None);
        assert_eq!(parse_positive(Some("many")), None);
        assert_eq!(parse_positive(None), None);
    }

    #[cfg(feature = "llama-cpp")]
    #[tokio::test]
    async fn waiting_for_an_unregistered_model_fails_at_once() {
        let pool = ContextPool::with_max_contexts(1);
        let started = std::time::Instant::now();
        let result = pool
            .acquire_fresh_within("missing", Duration::from_secs(30))
            .await;
        assert!(matches!(result, Err(Error::Config(_))));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
