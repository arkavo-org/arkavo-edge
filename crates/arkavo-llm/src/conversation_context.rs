//! Conversation Context Manager - Per-Conversation KV Cache Preservation
//!
//! Manages LlamaContext instances per conversation, enabling:
//! - KV cache continuity across multi-turn conversations
//! - Efficient context reuse without clearing cache between turns
//! - Automatic cleanup of idle conversation contexts

use std::collections::HashMap;
use std::sync::RwLock;

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use std::time::{Duration, Instant};

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use crate::context_pool::{ContextPool, PooledContext};
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use crate::{Error, Result};
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use std::sync::Arc;

/// Unique identifier for a conversation session
pub type ConversationId = String;

/// Tracks a context assigned to a specific conversation
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub struct ConversationContext {
    /// The pooled context for this conversation
    pub context: PooledContext,
    /// Conversation identifier
    pub conversation_id: ConversationId,
    /// Last time this context was used
    pub last_used: Instant,
    /// Current token position in KV cache (for resuming generation)
    pub token_position: i32,
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
impl ConversationContext {
    fn new(context: PooledContext, conversation_id: ConversationId) -> Self {
        Self {
            context,
            conversation_id,
            last_used: Instant::now(),
            token_position: 0,
        }
    }

    /// Update the last used timestamp
    pub fn touch(&mut self) {
        self.last_used = Instant::now();
    }

    /// Update the token position after generation
    pub fn set_token_position(&mut self, pos: i32) {
        self.token_position = pos;
    }
}

/// Stub for non-llama-cpp builds
#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
pub struct ConversationContext {
    pub conversation_id: ConversationId,
    pub token_position: i32,
}

/// Manages conversation-to-context mappings with automatic idle cleanup
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub struct ConversationContextManager {
    /// Active conversation contexts indexed by conversation ID
    active: RwLock<HashMap<ConversationId, ConversationContext>>,
    /// Context pool for acquiring/releasing contexts
    pool: Arc<ContextPool>,
    /// How long a context can be idle before cleanup
    idle_timeout: Duration,
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
impl ConversationContextManager {
    /// Create a new manager with the given context pool
    pub fn new(pool: Arc<ContextPool>) -> Self {
        Self::with_timeout(pool, Duration::from_mins(5))
    }

    /// Create a new manager with custom idle timeout
    pub fn with_timeout(pool: Arc<ContextPool>, idle_timeout: Duration) -> Self {
        Self {
            active: RwLock::new(HashMap::new()),
            pool,
            idle_timeout,
        }
    }

    /// Get or create a context for the given conversation
    ///
    /// If the conversation already has an active context, returns it.
    /// Otherwise, acquires a fresh context from the pool.
    #[allow(clippy::significant_drop_tightening)]
    pub fn get_context(
        &self,
        model_name: &str,
        conversation_id: &ConversationId,
    ) -> Result<ConversationContextRef> {
        // Check if we already have a context for this conversation
        {
            let active = self
                .active
                .read()
                .map_err(|_| Error::Internal("Lock poisoned".to_string()))?;

            if active.contains_key(conversation_id) {
                // Context exists, return a reference to it
                return Ok(ConversationContextRef {
                    conversation_id: conversation_id.clone(),
                    is_new: false,
                });
            }
        }

        // Need to acquire a new context
        let context = self.pool.acquire_fresh(model_name)?;
        let conv_ctx = ConversationContext::new(context, conversation_id.clone());

        {
            let mut active = self
                .active
                .write()
                .map_err(|_| Error::Internal("Lock poisoned".to_string()))?;
            active.insert(conversation_id.clone(), conv_ctx);
        }

        Ok(ConversationContextRef {
            conversation_id: conversation_id.clone(),
            is_new: true,
        })
    }

    /// Get an existing context for a conversation (does not create new)
    #[allow(clippy::significant_drop_tightening)]
    pub fn get_existing(&self, conversation_id: &ConversationId) -> Option<i32> {
        self.active
            .read()
            .ok()
            .and_then(|active| active.get(conversation_id).map(|ctx| ctx.token_position))
    }

    /// Update the token position for a conversation
    #[allow(clippy::significant_drop_tightening)]
    pub fn update_position(&self, conversation_id: &ConversationId, position: i32) -> Result<()> {
        let mut active = self
            .active
            .write()
            .map_err(|_| Error::Internal("Lock poisoned".to_string()))?;

        if let Some(ctx) = active.get_mut(conversation_id) {
            ctx.set_token_position(position);
            ctx.touch();
        }
        Ok(())
    }

    /// Access the underlying context for a conversation
    #[allow(clippy::significant_drop_tightening)]
    pub fn with_context<F, T>(&self, conversation_id: &ConversationId, f: F) -> Result<T>
    where
        F: FnOnce(&PooledContext, i32) -> Result<T>,
    {
        let active = self
            .active
            .read()
            .map_err(|_| Error::Internal("Lock poisoned".to_string()))?;

        let ctx = active.get(conversation_id).ok_or_else(|| {
            Error::Config(format!(
                "No context found for conversation '{conversation_id}'"
            ))
        })?;

        f(&ctx.context, ctx.token_position)
    }

    /// Release a conversation's context back to the pool
    #[allow(clippy::significant_drop_tightening)]
    pub fn release_conversation(&self, conversation_id: &ConversationId) -> Result<()> {
        let context = {
            let mut active = self
                .active
                .write()
                .map_err(|_| Error::Internal("Lock poisoned".to_string()))?;

            active.remove(conversation_id)
        };

        if let Some(ctx) = context {
            // Release back to pool, clearing the KV cache
            let model_name = ctx.context.model_name.clone();
            self.pool.release(&model_name, ctx.context, true)?;
        }

        Ok(())
    }

    /// Clean up contexts that have been idle longer than the timeout
    ///
    /// Returns the number of contexts cleaned up
    #[allow(clippy::significant_drop_tightening)]
    pub fn cleanup_idle(&self) -> usize {
        let now = Instant::now();
        let mut to_remove = Vec::new();

        // Find idle conversations
        if let Ok(active) = self.active.read() {
            for (id, ctx) in active.iter() {
                if now.duration_since(ctx.last_used) > self.idle_timeout {
                    to_remove.push(id.clone());
                }
            }
        }

        // Remove them
        let mut removed = 0;
        for id in to_remove {
            if self.release_conversation(&id).is_ok() {
                removed += 1;
            }
        }

        removed
    }

    /// Get the number of active conversation contexts
    pub fn active_count(&self) -> usize {
        self.active.read().ok().map(|a| a.len()).unwrap_or(0)
    }
}

/// Reference to a conversation context (used for tracking if context is new)
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub struct ConversationContextRef {
    pub conversation_id: ConversationId,
    pub is_new: bool,
}

/// Stub implementation for non-llama-cpp builds
#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
pub struct ConversationContextManager {
    _active: RwLock<HashMap<ConversationId, ConversationContext>>,
}

#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
impl ConversationContextManager {
    pub fn active_count(&self) -> usize {
        0
    }

    pub fn cleanup_idle(&self) -> usize {
        0
    }
}

#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
pub struct ConversationContextRef {
    pub conversation_id: ConversationId,
    pub is_new: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conversation_id_type() {
        let id: ConversationId = "test-conv-123".to_string();
        assert_eq!(id, "test-conv-123");
    }

    #[test]
    fn test_conversation_context_ref() {
        let ctx_ref = ConversationContextRef {
            conversation_id: "test".to_string(),
            is_new: true,
        };
        assert!(ctx_ref.is_new);
        assert_eq!(ctx_ref.conversation_id, "test");
    }

    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    #[test]
    fn test_manager_thread_safety() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ConversationContextManager>();
    }
}
