//! Context Pool - Per-Conversation Context Management
//!
//! Manages multiple LlamaContext instances per model, enabling:
//! - True concurrent inference (different contexts = parallel execution)
//! - KV cache isolation (each conversation has private cache)
//! - Efficient context reuse (pool returns cleared caches)

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::{Error, Result};

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
#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
pub struct ContextPool {
    pools: RwLock<HashMap<String, ()>>,
    default_max_contexts: usize,
}

#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
impl ContextPool {
    pub fn new() -> Self {
        Self::with_max_contexts(4)
    }

    pub fn with_max_contexts(max_contexts: usize) -> Self {
        Self {
            pools: RwLock::new(HashMap::new()),
            default_max_contexts: max_contexts,
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

#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
impl Default for ContextPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::{LlamaContext, LlamaModel};
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use std::collections::VecDeque;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use std::sync::Mutex;

/// A pooled context with its associated model
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub struct PooledContext {
    pub context: Arc<Mutex<LlamaContext>>,
    pub model_name: String,
    pub created_at: std::time::Instant,
    pub use_count: usize,
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
impl PooledContext {
    fn new(context: LlamaContext, model_name: String) -> Self {
        Self {
            context: Arc::new(Mutex::new(context)),
            model_name,
            created_at: std::time::Instant::now(),
            use_count: 0,
        }
    }

    /// Clear the KV cache to prepare for a new conversation
    pub fn clear_kv_cache(&self) {
        if let Ok(ctx) = self.context.lock() {
            ctx.clear_kv_cache();
        }
    }

    fn mark_used(&mut self) {
        self.use_count += 1;
    }
}

/// Pool of contexts for a specific model
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
struct ModelContextPool {
    model: Arc<LlamaModel>,
    available: VecDeque<PooledContext>,
    in_use: usize,
    max_contexts: usize,
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
impl ModelContextPool {
    fn new(model: Arc<LlamaModel>, max_contexts: usize) -> Self {
        Self {
            model,
            available: VecDeque::new(),
            in_use: 0,
            max_contexts,
        }
    }

    fn acquire(&mut self) -> Result<PooledContext> {
        // Try to get an available context
        if let Some(mut context) = self.available.pop_front() {
            context.clear_kv_cache();
            context.mark_used();
            self.in_use += 1;
            return Ok(context);
        }

        // Check if we can create more
        let total_contexts = self.in_use + self.available.len();
        if total_contexts >= self.max_contexts {
            return Err(Error::Internal(format!(
                "Max contexts ({}) reached for model. All contexts in use.",
                self.max_contexts
            )));
        }

        // Create new context
        let context = LlamaContext::new(&self.model)
            .map_err(|e| Error::Config(format!("Failed to create context: {e}")))?;

        self.in_use += 1;
        Ok(PooledContext::new(context, self.model_name()))
    }

    fn release(&mut self, context: PooledContext) {
        if self.in_use > 0 {
            self.in_use -= 1;
        }
        context.clear_kv_cache();
        self.available.push_back(context);
    }

    fn model_name(&self) -> String {
        self.model.model_name().to_string()
    }

    fn stats(&self) -> PoolStats {
        PoolStats {
            available: self.available.len(),
            in_use: self.in_use,
            max: self.max_contexts,
        }
    }
}

/// Manages pools of contexts for multiple models
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub struct ContextPool {
    pools: RwLock<HashMap<String, ModelContextPool>>,
    default_max_contexts: usize,
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
impl ContextPool {
    pub fn new() -> Self {
        Self::with_max_contexts(4)
    }

    pub fn with_max_contexts(max_contexts: usize) -> Self {
        Self {
            pools: RwLock::new(HashMap::new()),
            default_max_contexts: max_contexts,
        }
    }

    #[allow(clippy::significant_drop_tightening)]
    pub fn register_model(&self, name: &str, model: Arc<LlamaModel>) -> Result<()> {
        self.pools
            .write()
            .map_err(|_| Error::Internal("Pool lock poisoned".to_string()))?
            .insert(
                name.to_string(),
                ModelContextPool::new(model, self.default_max_contexts),
            );
        Ok(())
    }

    #[allow(clippy::significant_drop_tightening)]
    pub fn acquire(&self, model_name: &str) -> Result<PooledContext> {
        self.pools
            .write()
            .map_err(|_| Error::Internal("Pool lock poisoned".to_string()))?
            .get_mut(model_name)
            .ok_or_else(|| Error::Config(format!("Model '{model_name}' not registered in pool")))
            .and_then(|pool| pool.acquire())
    }

    #[allow(clippy::significant_drop_tightening)]
    pub fn release(&self, model_name: &str, context: PooledContext) -> Result<()> {
        let mut pools = self
            .pools
            .write()
            .map_err(|_| Error::Internal("Pool lock poisoned".to_string()))?;

        pools
            .get_mut(model_name)
            .map(|pool| pool.release(context))
            .ok_or_else(|| Error::Config(format!("Model '{model_name}' not found")))
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

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
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
        let pool: ContextPool = Default::default();
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
}
