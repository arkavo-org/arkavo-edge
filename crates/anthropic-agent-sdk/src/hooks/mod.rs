//! Hook system for intercepting agent events
//!
//! This module provides the hook system that allows users to intercept
//! and respond to various events in the agent lifecycle.
//!
//! # Architecture
//!
//! The hook system mirrors the TypeScript SDK design:
//! - Hooks are registered by event type (`PreToolUse`, `PostToolUse`, etc.)
//! - Each event type can have multiple matchers with patterns
//! - Matchers filter by tool name (e.g., "Bash", "Write|Edit", "*")
//! - Callbacks receive typed input and return `HookOutput`
//!
//! # Example
//!
//! ```no_run
//! use anthropic_agent_sdk::hooks::{HookManager, HookMatcherBuilder};
//! use anthropic_agent_sdk::types::{HookEvent, HookOutput, HookContext};
//! use std::collections::HashMap;
//!
//! let mut manager = HookManager::new();
//!
//! // Register a PreToolUse hook for Bash commands
//! let hook = HookManager::callback(|input, tool_name, ctx| async move {
//!     // ctx contains session_id, cwd, and cancellation_token
//!     println!("Tool: {:?}, Input: {:?}, Session: {:?}", tool_name, input, ctx.session_id);
//!     Ok(HookOutput::default())
//! });
//!
//! manager.register_for_event(
//!     HookEvent::PreToolUse,
//!     HookMatcherBuilder::new(Some("Bash")).add_hook(hook).build(),
//! );
//! ```

mod invocation;
mod processing;

use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::callbacks::{FnHookCallback, HookCallback};
use crate::error::Result;
use crate::types::{ContentValue, HookContext, HookEvent, HookMatcher, HookOutput};

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert `ContentValue` to a string representation for error messages
pub(crate) fn content_to_string(content: Option<&ContentValue>) -> String {
    match content {
        Some(ContentValue::String(s)) => s.clone(),
        Some(ContentValue::Blocks(blocks)) => {
            // Try to extract text from blocks
            blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        }
        None => String::new(),
    }
}

// ============================================================================
// Pending Tool Use Tracking
// ============================================================================

/// Tracks a tool use for matching with its result
#[derive(Debug, Clone)]
pub struct PendingToolUse {
    /// Tool name
    pub tool_name: String,
    /// Tool input
    pub tool_input: serde_json::Value,
    /// Whether this is a Task (subagent) tool
    pub is_task: bool,
}

// ============================================================================
// Hook Manager
// ============================================================================

/// Hook manager for registering and invoking hooks by event type
///
/// Mirrors the TypeScript SDK structure:
/// ```typescript
/// hooks: Partial<Record<HookEvent, HookCallbackMatcher[]>>
/// ```
pub struct HookManager {
    /// Hooks registered by event type
    pub(crate) hooks_by_event: HashMap<HookEvent, Vec<HookMatcher>>,
    /// Pending tool uses awaiting results (`tool_use_id` -> info)
    pub(crate) pending_tools: HashMap<String, PendingToolUse>,
    /// Session context for constructing hook inputs
    pub(crate) session_id: Option<String>,
    /// Current working directory
    pub(crate) cwd: Option<String>,
    /// Cancellation token for aborting operations
    pub(crate) cancellation_token: Option<CancellationToken>,
    /// Current subagent context (`parent_tool_use_id` of active subagent)
    /// When set, we're processing messages from inside a subagent.
    /// When a message with `parent_tool_use_id=None` arrives after this is set,
    /// the subagent has completed and we trigger `SubagentStop`.
    pub(crate) current_subagent: Option<String>,
}

impl HookManager {
    /// Create a new hook manager
    #[must_use]
    pub fn new() -> Self {
        Self {
            hooks_by_event: HashMap::new(),
            pending_tools: HashMap::new(),
            session_id: None,
            cwd: None,
            cancellation_token: None,
            current_subagent: None,
        }
    }

    /// Create from options hooks configuration
    ///
    /// This is the primary constructor matching TypeScript SDK pattern.
    #[must_use]
    pub fn from_hooks_config(config: HashMap<HookEvent, Vec<HookMatcher>>) -> Self {
        Self {
            hooks_by_event: config,
            pending_tools: HashMap::new(),
            session_id: None,
            cwd: None,
            cancellation_token: None,
            current_subagent: None,
        }
    }

    /// Set session context (called when system init message is received)
    pub fn set_session_context(&mut self, session_id: String, cwd: Option<String>) {
        self.session_id = Some(session_id);
        self.cwd = cwd;
    }

    /// Set cancellation token for aborting operations
    pub fn set_cancellation_token(&mut self, token: CancellationToken) {
        self.cancellation_token = Some(token);
    }

    /// Build a `HookContext` with current session information
    #[must_use]
    pub fn build_context(&self) -> HookContext {
        HookContext::new(
            self.session_id.clone(),
            self.cwd.clone(),
            self.cancellation_token.clone(),
        )
    }

    /// Register a hook matcher for a specific event type
    pub fn register_for_event(&mut self, event: HookEvent, matcher: HookMatcher) {
        self.hooks_by_event.entry(event).or_default().push(matcher);
    }

    /// Register a hook with a matcher (legacy API - registers for all events)
    #[deprecated(note = "Use register_for_event() instead")]
    pub fn register(&mut self, matcher: HookMatcher) {
        // For backward compatibility, register for PreToolUse
        self.register_for_event(HookEvent::PreToolUse, matcher);
    }

    /// Check if there are any hooks registered for a given event
    #[must_use]
    pub fn has_hooks_for(&self, event: HookEvent) -> bool {
        self.hooks_by_event
            .get(&event)
            .is_some_and(|v| !v.is_empty())
    }

    // ========================================================================
    // Pattern Matching
    // ========================================================================

    /// Check if a matcher matches a tool name
    ///
    /// # Security Note
    /// This uses simple pattern matching with pipe-separated alternatives.
    /// For production use with untrusted patterns, consider using a proper
    /// glob or regex library with safety guarantees (e.g., `globset` crate).
    pub(crate) fn matches(matcher: Option<&String>, tool_name: Option<&String>) -> bool {
        match (matcher, tool_name) {
            (None, _) => true, // No matcher = match all
            (Some(pattern), Some(name)) => {
                // Simple wildcard matching
                if pattern == "*" {
                    return true;
                }
                // Exact match or simple pipe-separated pattern
                // Note: This doesn't handle edge cases like pipe characters in tool names
                pattern == name || pattern.split('|').any(|p| p == name)
            }
            (Some(_), None) => false,
        }
    }

    // ========================================================================
    // Callback Helpers
    // ========================================================================

    /// Create a hook callback from a closure
    ///
    /// # Example
    ///
    /// ```no_run
    /// use anthropic_agent_sdk::{HookManager, HookOutput, HookContext};
    ///
    /// let hook = HookManager::callback(|_input, tool_name, ctx| async move {
    ///     // Check cancellation and access session info
    ///     if ctx.is_cancelled() { return Ok(HookOutput::default()); }
    ///     println!("Tool: {:?}, Session: {:?}", tool_name, ctx.session_id);
    ///     Ok(HookOutput::default())
    /// });
    /// ```
    pub fn callback<F, Fut>(f: F) -> Arc<dyn HookCallback>
    where
        F: Fn(serde_json::Value, Option<String>, HookContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<HookOutput>> + Send + 'static,
    {
        Arc::new(FnHookCallback::new(
            move |event_data, tool_name, context| Box::pin(f(event_data, tool_name, context)),
        ))
    }
}

impl Default for HookManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for creating hook matchers
pub struct HookMatcherBuilder {
    matcher: Option<String>,
    hooks: Vec<Arc<dyn HookCallback>>,
    timeout: Option<std::time::Duration>,
}

impl HookMatcherBuilder {
    /// Create a new hook matcher builder
    ///
    /// # Arguments
    /// * `pattern` - Matcher pattern (None for all, or specific tool name/pattern)
    pub fn new(pattern: Option<impl Into<String>>) -> Self {
        Self {
            matcher: pattern.map(std::convert::Into::into),
            hooks: Vec::new(),
            timeout: None,
        }
    }

    /// Add a hook callback
    #[must_use]
    pub fn add_hook(mut self, hook: Arc<dyn HookCallback>) -> Self {
        self.hooks.push(hook);
        self
    }

    /// Set timeout for all hooks in this matcher
    ///
    /// Default is 60 seconds if not specified.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use anthropic_agent_sdk::hooks::HookMatcherBuilder;
    /// use std::time::Duration;
    ///
    /// let matcher = HookMatcherBuilder::new(Some("Bash"))
    ///     .timeout(Duration::from_secs(30))
    ///     .build();
    /// ```
    #[must_use]
    pub fn timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Build the hook matcher
    #[must_use]
    pub fn build(self) -> HookMatcher {
        HookMatcher {
            matcher: self.matcher,
            hooks: self.hooks,
            timeout: self.timeout,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_hook_manager() {
        let mut manager = HookManager::new();

        // Register a hook for PreToolUse
        let hook = HookManager::callback(|_event_data, _tool_name, _context| async {
            Ok(HookOutput::default())
        });

        let matcher = HookMatcherBuilder::new(Some("*")).add_hook(hook).build();
        manager.register_for_event(HookEvent::PreToolUse, matcher);

        // Invoke hook
        let result = manager
            .invoke(
                HookEvent::PreToolUse,
                serde_json::json!({}),
                Some("test".to_string()),
                HookContext::default(),
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_hook_manager_by_event() {
        let mut manager = HookManager::new();

        // Register hooks for different events - ctx provides session info
        let pre_hook = HookManager::callback(|_data, _tool, ctx| async move {
            // Verify ctx fields are accessible (use references to avoid move)
            let _ = (&ctx.session_id, &ctx.cwd, ctx.is_cancelled());
            Ok(HookOutput {
                system_message: Some("pre".to_string()),
                ..Default::default()
            })
        });
        let post_hook = HookManager::callback(|_data, _tool, ctx| async move {
            let _ = &ctx.cancellation_token;
            Ok(HookOutput {
                system_message: Some("post".to_string()),
                ..Default::default()
            })
        });

        manager.register_for_event(
            HookEvent::PreToolUse,
            HookMatcherBuilder::new(None::<String>)
                .add_hook(pre_hook)
                .build(),
        );
        manager.register_for_event(
            HookEvent::PostToolUse,
            HookMatcherBuilder::new(None::<String>)
                .add_hook(post_hook)
                .build(),
        );

        // PreToolUse should return "pre"
        let result = manager
            .invoke(
                HookEvent::PreToolUse,
                serde_json::json!({}),
                None,
                HookContext::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.system_message, Some("pre".to_string()));

        // PostToolUse should return "post"
        let result = manager
            .invoke(
                HookEvent::PostToolUse,
                serde_json::json!({}),
                None,
                HookContext::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.system_message, Some("post".to_string()));

        // SubagentStart should return empty (no hooks registered)
        let result = manager
            .invoke(
                HookEvent::SubagentStart,
                serde_json::json!({}),
                None,
                HookContext::default(),
            )
            .await
            .unwrap();
        assert!(result.system_message.is_none());
    }

    #[test]
    fn test_matcher_wildcard() {
        assert!(HookManager::matches(
            Some("*".to_string()).as_ref(),
            Some("any_tool".to_string()).as_ref()
        ));
        assert!(HookManager::matches(
            None,
            Some("any_tool".to_string()).as_ref()
        ));
    }

    #[test]
    fn test_matcher_specific() {
        assert!(HookManager::matches(
            Some("Bash".to_string()).as_ref(),
            Some("Bash".to_string()).as_ref()
        ));
        assert!(!HookManager::matches(
            Some("Bash".to_string()).as_ref(),
            Some("Write".to_string()).as_ref()
        ));
    }

    #[test]
    fn test_matcher_pattern() {
        assert!(HookManager::matches(
            Some("Write|Edit".to_string()).as_ref(),
            Some("Write".to_string()).as_ref()
        ));
        assert!(HookManager::matches(
            Some("Write|Edit".to_string()).as_ref(),
            Some("Edit".to_string()).as_ref()
        ));
        assert!(!HookManager::matches(
            Some("Write|Edit".to_string()).as_ref(),
            Some("Bash".to_string()).as_ref()
        ));
    }

    // ========================================================================
    // Security: Timeout Tests
    // ========================================================================

    #[tokio::test]
    async fn test_hook_timeout_prevents_blocking() {
        let mut manager = HookManager::new();

        // Create a hook that would block forever without timeout
        let slow_hook = HookManager::callback(|_data, _tool, _ctx| async move {
            // Simulate a very slow operation
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            Ok(HookOutput {
                system_message: Some("should never see this".to_string()),
                ..Default::default()
            })
        });

        // Register with a very short timeout (100ms)
        let matcher = HookMatcherBuilder::new(Some("*"))
            .timeout(std::time::Duration::from_millis(100))
            .add_hook(slow_hook)
            .build();
        manager.register_for_event(HookEvent::PreToolUse, matcher);

        // Should complete quickly (timeout) and return default output
        let start = std::time::Instant::now();
        let result = manager
            .invoke(
                HookEvent::PreToolUse,
                serde_json::json!({}),
                Some("test".to_string()),
                HookContext::default(),
            )
            .await;

        let elapsed = start.elapsed();

        // Verify timeout was enforced (should be ~100ms, not 10s)
        assert!(elapsed < std::time::Duration::from_secs(1));
        assert!(result.is_ok());

        // Timed-out hook returns default output (no system_message)
        let output = result.unwrap();
        assert!(output.system_message.is_none());
        assert!(output.decision.is_none());
    }

    #[tokio::test]
    async fn test_hook_custom_timeout_respected() {
        let mut manager = HookManager::new();

        // Hook that takes 200ms
        let medium_hook = HookManager::callback(|_data, _tool, _ctx| async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            Ok(HookOutput {
                system_message: Some("completed".to_string()),
                ..Default::default()
            })
        });

        // Register with 500ms timeout - should complete
        let matcher = HookMatcherBuilder::new(Some("*"))
            .timeout(std::time::Duration::from_millis(500))
            .add_hook(medium_hook)
            .build();
        manager.register_for_event(HookEvent::PreToolUse, matcher);

        let result = manager
            .invoke(
                HookEvent::PreToolUse,
                serde_json::json!({}),
                Some("test".to_string()),
                HookContext::default(),
            )
            .await
            .unwrap();

        // Hook should complete within timeout
        assert_eq!(result.system_message, Some("completed".to_string()));
    }

    #[tokio::test]
    async fn test_hook_default_timeout_is_60_seconds() {
        // Verify the constant is set correctly
        assert_eq!(
            HookManager::DEFAULT_HOOK_TIMEOUT,
            std::time::Duration::from_secs(60)
        );

        // Verify builder without timeout uses None (which means default)
        let matcher = HookMatcherBuilder::new(Some("*")).build();
        assert!(matcher.timeout.is_none());
    }

    #[tokio::test]
    async fn test_fast_hook_unaffected_by_timeout() {
        let mut manager = HookManager::new();

        // Fast hook that completes immediately
        let fast_hook = HookManager::callback(|_data, _tool, _ctx| async move {
            Ok(HookOutput {
                system_message: Some("fast".to_string()),
                ..Default::default()
            })
        });

        // Even with short timeout, fast hook completes
        let matcher = HookMatcherBuilder::new(Some("*"))
            .timeout(std::time::Duration::from_millis(100))
            .add_hook(fast_hook)
            .build();
        manager.register_for_event(HookEvent::PreToolUse, matcher);

        let result = manager
            .invoke(
                HookEvent::PreToolUse,
                serde_json::json!({}),
                Some("test".to_string()),
                HookContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(result.system_message, Some("fast".to_string()));
    }

    // ========================================================================
    // Security: Cancellation Token Tests
    // ========================================================================

    #[tokio::test]
    async fn test_hook_context_cancellation() {
        let token = CancellationToken::new();
        let ctx = HookContext::new(
            Some("session-1".to_string()),
            Some("/tmp".to_string()),
            Some(token.clone()),
        );

        // Initially not cancelled
        assert!(!ctx.is_cancelled());

        // Cancel the token
        token.cancel();

        // Now should be cancelled
        assert!(ctx.is_cancelled());
    }

    #[tokio::test]
    async fn test_hook_receives_cancellation_token() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let mut manager = HookManager::new();
        let was_cancelled = Arc::new(AtomicBool::new(false));
        let was_cancelled_clone = was_cancelled.clone();

        // Hook that checks cancellation status
        let hook = HookManager::callback(move |_data, _tool, ctx| {
            let was_cancelled = was_cancelled_clone.clone();
            async move {
                // Store whether we received a cancellation token
                if ctx.cancellation_token.is_some() {
                    was_cancelled.store(true, Ordering::SeqCst);
                }
                Ok(HookOutput::default())
            }
        });

        let matcher = HookMatcherBuilder::new(Some("*")).add_hook(hook).build();
        manager.register_for_event(HookEvent::PreToolUse, matcher);

        // Invoke with cancellation token
        let token = CancellationToken::new();
        let ctx = HookContext::new(None, None, Some(token));

        let _ = manager
            .invoke(
                HookEvent::PreToolUse,
                serde_json::json!({}),
                Some("test".to_string()),
                ctx,
            )
            .await;

        assert!(was_cancelled.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_hook_context_default_has_no_token() {
        let ctx = HookContext::default();
        assert!(ctx.cancellation_token.is_none());
        assert!(!ctx.is_cancelled()); // No token means not cancelled
    }
}
