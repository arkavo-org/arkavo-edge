//! Conversation management for chat sessions
//!
//! Provides session-based conversation tracking with message history,
//! token counting, and automatic summarization support.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

use arkavo_llm::{LlmClient, Message};
use arkavo_memory::models::Memory;
use arkavo_memory::storage::MemoryStorage;
use indicatif::{ProgressBar, ProgressStyle};
use tiktoken_rs::{CoreBPE, cl100k_base};

/// Debug flag for conversation manager
static SHOW_DEBUG: AtomicBool = AtomicBool::new(false);

/// Set debug mode for conversation manager
pub fn set_debug(enabled: bool) {
    SHOW_DEBUG.store(enabled, Ordering::SeqCst);
}

/// Check if debug mode is enabled
pub fn is_debug() -> bool {
    SHOW_DEBUG.load(Ordering::Relaxed)
}

const MAX_CONTEXT_MESSAGES: usize = 10;
const MAX_CONTEXT_TOKENS: usize = 1500;
const SUMMARY_TRIGGER_MESSAGES: usize = 10;

/// A single message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    /// Unique message identifier
    pub id: Uuid,
    /// Session this message belongs to
    pub session_id: Uuid,
    /// Role: "user", "assistant", or "system"
    pub role: String,
    /// Message content
    pub content: String,
    /// When the message was created
    pub timestamp: DateTime<Utc>,
    /// Estimated token count
    pub token_count: usize,
    /// Whether this is a summary message
    pub is_summary: bool,
}

/// A conversation session containing multiple messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSession {
    /// Unique session identifier
    pub id: Uuid,
    /// When the session was created
    pub created_at: DateTime<Utc>,
    /// When the session was last updated
    pub updated_at: DateTime<Utc>,
    /// Model used for this session
    pub model: String,
    /// Optional session title
    pub title: Option<String>,
    /// Hash of chat template for compatibility checking
    pub chat_template_hash: Option<String>,
    /// Hash of system prompt for compatibility checking
    pub system_prompt_hash: Option<String>,
    /// Model size hint (e.g., "270M", "1B", "7B")
    pub model_size_hint: Option<String>,
}

/// Manages conversation sessions with persistent storage
pub struct ConversationManager {
    memory_storage: Arc<MemoryStorage>,
    token_encoder: CoreBPE,
    current_session_id: Option<Uuid>,
}

impl ConversationManager {
    /// Create a new conversation manager with memory storage
    pub fn new(memory_storage: Arc<MemoryStorage>) -> anyhow::Result<Self> {
        Ok(Self {
            memory_storage,
            token_encoder: cl100k_base()?,
            current_session_id: None,
        })
    }

    /// Get the current session ID if one is active
    pub const fn current_session_id(&self) -> Option<Uuid> {
        self.current_session_id
    }

    /// Alias for current_session_id (backwards compatibility)
    pub const fn get_current_session_id(&self) -> Option<Uuid> {
        self.current_session_id
    }

    /// Sanitize message content to prevent generation issues with small models
    pub fn sanitize_message_content(content: &str) -> String {
        let mut sanitized = content.to_string();

        // Count and balance triple backticks
        let fence_count = sanitized.matches("```").count();
        if !fence_count.is_multiple_of(2) {
            // Odd number of fences - close the last one
            sanitized.push_str("\n```\n");
        }

        // Remove trailing "Assistant:" or "Model:" artifacts
        let trimmed = sanitized.trim_end();
        if trimmed.ends_with("Assistant:")
            || trimmed.ends_with("Model:")
            || trimmed.ends_with("User:")
        {
            sanitized = format!("{trimmed}\n");
        }

        // Add buffer after "code:" patterns to prevent immediate fence generation
        if sanitized.trim_end().ends_with("code:")
            || sanitized.trim_end().ends_with("Here's the code:")
            || sanitized.trim_end().ends_with("```python")
        {
            sanitized.push_str("\n\n");
        }

        sanitized
    }

    /// Calculate a hash for the given string (for template/prompt comparison)
    fn calculate_hash(s: &str) -> String {
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    /// Start a new session with just a model name
    pub async fn start_session(&mut self, model: &str) -> anyhow::Result<Uuid> {
        self.start_session_with_metadata(model, None, None, None)
            .await
    }

    /// Start a new session with full metadata
    pub async fn start_session_with_metadata(
        &mut self,
        model: &str,
        chat_template: Option<&str>,
        system_prompt: Option<&str>,
        model_size: Option<String>,
    ) -> anyhow::Result<Uuid> {
        let session_id = Uuid::new_v4();
        let session = ConversationSession {
            id: session_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            model: model.to_string(),
            title: None,
            chat_template_hash: chat_template.map(Self::calculate_hash),
            system_prompt_hash: system_prompt.map(Self::calculate_hash),
            model_size_hint: model_size,
        };

        // Store session metadata
        let memory = Memory {
            id: Uuid::new_v4(),
            content: serde_json::to_string(&session)?,
            metadata: Some(json!({
                "type": "conversation_session",
                "session_id": session_id,
                "model": model,
            })),
            category: Some("conversation".to_string()),
            embedding: vec![0.0; 384],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.memory_storage.store(memory).await?;
        self.current_session_id = Some(session_id);
        Ok(session_id)
    }

    /// Restore the last session if compatible
    pub async fn restore_last_session(&mut self) -> anyhow::Result<Option<Uuid>> {
        self.restore_last_session_with_compatibility(None, None, None)
            .await
    }

    /// Restore last session with compatibility checking
    #[allow(
        clippy::missing_panics_doc,
        clippy::literal_string_with_formatting_args
    )]
    pub async fn restore_last_session_with_compatibility(
        &mut self,
        current_template: Option<&str>,
        current_prompt: Option<&str>,
        current_model: Option<&str>,
    ) -> anyhow::Result<Option<Uuid>> {
        let progress = ProgressBar::new_spinner();
        progress.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        progress.set_message("Restoring last conversation...");

        // Query for recent conversation sessions
        let sessions = self
            .memory_storage
            .search("conversation_session", 10, Some("conversation"))
            .await?;

        if let Some(latest_session) = sessions.first()
            && let Ok(session) =
                serde_json::from_str::<ConversationSession>(&latest_session.memory.content)
        {
            // Check compatibility if metadata provided
            let mut compatible = true;
            let mut incompatibility_reason = String::new();

            // Check template hash compatibility
            if let (Some(current_tmpl), Some(session_tmpl_hash)) =
                (current_template, &session.chat_template_hash)
            {
                let current_hash = Self::calculate_hash(current_tmpl);
                if current_hash != *session_tmpl_hash {
                    compatible = false;
                    incompatibility_reason.push_str("template mismatch, ");
                }
            }

            // Check system prompt compatibility
            if let (Some(current_sys), Some(session_sys_hash)) =
                (current_prompt, &session.system_prompt_hash)
            {
                let current_hash = Self::calculate_hash(current_sys);
                if current_hash != *session_sys_hash {
                    compatible = false;
                    incompatibility_reason.push_str("system prompt mismatch, ");
                }
            }

            // Check model compatibility
            if let Some(current_mdl) = current_model {
                let current_base = current_mdl.split('-').take(2).collect::<Vec<_>>().join("-");
                let session_base = session
                    .model
                    .split('-')
                    .take(2)
                    .collect::<Vec<_>>()
                    .join("-");
                if current_base != session_base {
                    compatible = false;
                    incompatibility_reason.push_str("model mismatch, ");
                }
            }

            if compatible {
                self.current_session_id = Some(session.id);
                progress.finish_with_message(format!(
                    "Restored session from {}",
                    session.created_at.format("%Y-%m-%d %H:%M")
                ));
                return Ok(Some(session.id));
            }

            incompatibility_reason = incompatibility_reason.trim_end_matches(", ").to_string();
            progress.finish_with_message(format!(
                "Session skipped: {incompatibility_reason} (starting fresh)"
            ));
            return Ok(None);
        }

        progress.finish_and_clear();
        Ok(None)
    }

    /// Add a message to the current session
    pub async fn add_message(&self, message: &Message) -> anyhow::Result<()> {
        let session_id = self
            .current_session_id
            .ok_or_else(|| anyhow::anyhow!("No active conversation session"))?;

        let content = message.content.clone();
        let token_count = self.count_tokens(&content);

        let conv_message = ConversationMessage {
            id: Uuid::new_v4(),
            session_id,
            role: format!("{:?}", message.role).to_lowercase(),
            content: content.clone(),
            timestamp: Utc::now(),
            token_count,
            is_summary: false,
        };

        let memory = Memory {
            id: conv_message.id,
            content: serde_json::to_string(&conv_message)?,
            metadata: Some(json!({
                "type": "conversation_message",
                "session_id": session_id,
                "role": conv_message.role,
                "timestamp": conv_message.timestamp,
                "token_count": token_count,
            })),
            category: Some("conversation".to_string()),
            embedding: vec![0.0; 384],
            created_at: conv_message.timestamp,
            updated_at: conv_message.timestamp,
        };

        self.memory_storage.store(memory).await?;
        Ok(())
    }

    /// Add a message by role and content strings
    pub async fn add_message_str(&self, role: &str, content: &str) -> anyhow::Result<()> {
        let message = match role {
            "user" => Message::user(content),
            "assistant" => Message::assistant(content),
            "system" => Message::system(content),
            _ => return Err(anyhow::anyhow!("Invalid role: {role}")),
        };
        self.add_message(&message).await
    }

    /// Get context messages for the current session
    #[allow(
        clippy::missing_panics_doc,
        clippy::literal_string_with_formatting_args
    )]
    pub async fn get_context_messages(
        &self,
        system_message: Option<Message>,
    ) -> anyhow::Result<Vec<Message>> {
        self.get_context_messages_with_limits(system_message, None)
            .await
    }

    /// Get context messages with optional limits
    #[allow(
        clippy::literal_string_with_formatting_args,
        clippy::missing_panics_doc
    )]
    pub async fn get_context_messages_with_limits(
        &self,
        system_message: Option<Message>,
        max_history_turns: Option<usize>,
    ) -> anyhow::Result<Vec<Message>> {
        let session_id = self
            .current_session_id
            .ok_or_else(|| anyhow::anyhow!("No active conversation session"))?;

        let progress = ProgressBar::new_spinner();
        progress.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        progress.set_message("Loading conversation context...");

        // Query recent messages for this session
        let query = format!("session_id:{session_id}");
        let results = self
            .memory_storage
            .search(&query, 100, Some("conversation"))
            .await?;

        let mut messages: Vec<ConversationMessage> = results
            .into_iter()
            .filter_map(|result| {
                serde_json::from_str::<ConversationMessage>(&result.memory.content).ok()
            })
            .filter(|msg| msg.session_id == session_id)
            .collect();

        // Sort by timestamp
        messages.sort_by_key(|m| m.timestamp);

        // Apply sliding window
        let mut context_messages = Vec::new();
        if let Some(sys_msg) = system_message {
            context_messages.push(sys_msg);
        }

        // Check if we need summarization
        if messages.len() > SUMMARY_TRIGGER_MESSAGES {
            progress.set_message("Checking for conversation summary...");

            let summary_query = format!("session_id:{session_id} type:summary");
            let summaries = self
                .memory_storage
                .search(&summary_query, 5, Some("conversation"))
                .await?;

            if let Some(latest_summary) = summaries.first()
                && let Ok(summary_msg) =
                    serde_json::from_str::<ConversationMessage>(&latest_summary.memory.content)
            {
                context_messages.push(Message::assistant(&summary_msg.content));
            }
        }

        // Determine effective history limit
        let history_limit = max_history_turns.unwrap_or_else(|| {
            std::env::var("ARKAVO_MAX_HISTORY_TURNS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(MAX_CONTEXT_MESSAGES)
        });

        // Add recent messages within token budget and history limit
        let mut total_tokens = self.count_message_tokens(&context_messages);
        let recent_messages: Vec<_> = messages.iter().rev().take(history_limit).rev().collect();

        for (idx, msg) in recent_messages.iter().enumerate() {
            let msg_tokens = msg.token_count;
            if total_tokens + msg_tokens > MAX_CONTEXT_TOKENS {
                break;
            }

            let sanitized_content = if idx == recent_messages.len() - 1 {
                Self::sanitize_message_content(&msg.content)
            } else {
                msg.content.clone()
            };

            let message = match msg.role.as_str() {
                "user" => Message::user(&sanitized_content),
                "assistant" => Message::assistant(&sanitized_content),
                "system" => Message::system(&sanitized_content),
                _ => continue,
            };

            context_messages.push(message);
            total_tokens += msg_tokens;
        }

        if is_debug() {
            eprintln!(
                "Context messages: {} messages, {} tokens",
                context_messages.len(),
                total_tokens
            );
        }

        // Check fence parity
        let full_context = context_messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let fence_count = full_context.matches("```").count();
        if is_debug() {
            if fence_count % 2 != 0 {
                eprintln!("Fence parity: {fence_count} (UNBALANCED!)");
            } else {
                eprintln!("Fence parity: {fence_count} (balanced)");
            }
        }

        progress.finish_and_clear();
        Ok(context_messages)
    }

    /// Create a summary of conversation messages
    #[allow(
        clippy::missing_panics_doc,
        clippy::literal_string_with_formatting_args
    )]
    pub async fn create_summary(
        &self,
        client: &LlmClient,
        messages_to_summarize: Vec<ConversationMessage>,
    ) -> anyhow::Result<String> {
        let progress = ProgressBar::new_spinner();
        progress.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        progress.set_message("Creating conversation summary...");

        let mut summary_prompt = String::from(
            "Please provide a concise summary of the following conversation, \
             focusing on key decisions, important context, and any unresolved topics:\n\n",
        );

        for msg in &messages_to_summarize {
            use std::fmt::Write;
            let _ = write!(
                summary_prompt,
                "{}: {}\n\n",
                msg.role.to_uppercase(),
                msg.content
            );
        }

        let messages = vec![
            Message::system(
                "You are a helpful assistant that creates concise conversation summaries.",
            ),
            Message::user(&summary_prompt),
        ];

        let summary = client.complete(messages).await?;

        // Store the summary
        let session_id = self.current_session_id.unwrap();
        let summary_message = ConversationMessage {
            id: Uuid::new_v4(),
            session_id,
            role: "system".to_string(),
            content: format!("Previous conversation summary:\n{summary}"),
            timestamp: Utc::now(),
            token_count: self.count_tokens(&summary),
            is_summary: true,
        };

        let memory = Memory {
            id: summary_message.id,
            content: serde_json::to_string(&summary_message)?,
            metadata: Some(json!({
                "type": "conversation_summary",
                "session_id": session_id,
                "summarized_messages": messages_to_summarize.len(),
                "timestamp": summary_message.timestamp,
            })),
            category: Some("conversation".to_string()),
            embedding: vec![0.0; 384],
            created_at: summary_message.timestamp,
            updated_at: summary_message.timestamp,
        };

        self.memory_storage.store(memory).await?;
        progress.finish_with_message("Summary created");

        Ok(summary)
    }

    /// Clear the current session
    pub fn clear_session(&mut self) -> anyhow::Result<()> {
        self.current_session_id = None;
        Ok(())
    }

    /// Get statistics for the current session
    pub async fn get_session_stats(&self) -> anyhow::Result<(usize, usize, bool)> {
        let session_id = self
            .current_session_id
            .ok_or_else(|| anyhow::anyhow!("No active conversation session"))?;

        let session_str = session_id.to_string().replace('-', " ");
        let results = self
            .memory_storage
            .search(&session_str, 100, Some("conversation"))
            .await?;

        let messages: Vec<ConversationMessage> = results
            .into_iter()
            .filter_map(|result| {
                serde_json::from_str::<ConversationMessage>(&result.memory.content).ok()
            })
            .filter(|msg| msg.session_id == session_id)
            .collect();

        let turn_count = messages.len();
        let token_count: usize = messages.iter().map(|m| m.token_count).sum();

        let full_text = messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let fence_count = full_text.matches("```").count();
        let fence_balanced = fence_count % 2 == 0;

        Ok((turn_count, token_count, fence_balanced))
    }

    /// List all sessions
    pub async fn list_sessions(&self) -> anyhow::Result<Vec<ConversationSession>> {
        let results = self
            .memory_storage
            .search("conversation_session", 50, Some("conversation"))
            .await?;

        let mut sessions: Vec<ConversationSession> = results
            .into_iter()
            .filter_map(|result| {
                serde_json::from_str::<ConversationSession>(&result.memory.content).ok()
            })
            .collect();

        sessions.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
        Ok(sessions)
    }

    /// Switch to a different session
    pub async fn switch_session(&mut self, session_id: Uuid) -> anyhow::Result<()> {
        // Perform a targeted lookup for the session and accept it only if the
        // returned memory deserializes to a ConversationSession with a matching
        // id. This avoids both false positives from unrelated memories and
        // false negatives from a capped global list.
        let query = format!("session_id:{session_id}");
        let results = self
            .memory_storage
            .search(&query, 5, Some("conversation"))
            .await?;
        let found = results.into_iter().any(|result| {
            serde_json::from_str::<ConversationSession>(&result.memory.content)
                .is_ok_and(|session| session.id == session_id)
        });

        if found {
            self.current_session_id = Some(session_id);
            Ok(())
        } else {
            Err(anyhow::anyhow!("Session not found"))
        }
    }

    /// Count tokens in a string
    fn count_tokens(&self, text: &str) -> usize {
        self.token_encoder.encode_with_special_tokens(text).len()
    }

    /// Count tokens in a list of messages
    fn count_message_tokens(&self, messages: &[Message]) -> usize {
        messages
            .iter()
            .map(|msg| self.count_tokens(&msg.content))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    // Mock-provider support for summary tests
    use async_trait::async_trait;

    #[derive(Clone)]
    struct MockSummaryProvider {
        response: String,
    }

    #[async_trait]
    impl arkavo_llm::Provider for MockSummaryProvider {
        async fn complete_with_options(
            &self,
            _messages: Vec<arkavo_llm::Message>,
            _max_tokens: Option<usize>,
        ) -> arkavo_llm::Result<String> {
            Ok(self.response.clone())
        }

        async fn stream(
            &self,
            _messages: Vec<arkavo_llm::Message>,
        ) -> arkavo_llm::Result<
            Box<
                dyn futures::Stream<Item = arkavo_llm::Result<arkavo_llm::StreamResponse>>
                    + Send
                    + Unpin,
            >,
        > {
            Ok(Box::new(futures::stream::empty()))
        }

        fn name(&self) -> &str {
            "mock-summary-provider"
        }
    }

    #[test]
    #[spec("CHAT-024")]
    fn test_sanitize_message_content_balances_fences() {
        // Arrange - odd number of fences
        let content = "Here is code:\n```rust\nfn main() {}";

        // Act
        let sanitized = ConversationManager::sanitize_message_content(content);

        // Assert - should close unclosed fence
        assert!(sanitized.ends_with("```\n"));
        assert_eq!(sanitized.matches("```").count() % 2, 0);
    }

    #[test]
    #[spec("CHAT-024")]
    fn test_sanitize_message_content_even_fences_unchanged() {
        let content = "```rust\nfn main() {}\n```";
        let sanitized = ConversationManager::sanitize_message_content(content);
        assert!(!sanitized.ends_with("```\n```\n"));
        assert_eq!(sanitized.matches("```").count() % 2, 0);
    }

    #[test]
    #[spec("CHAT-024")]
    fn test_sanitize_removes_trailing_role_markers() {
        let content = "Some text\nAssistant:";
        let sanitized = ConversationManager::sanitize_message_content(content);
        assert!(sanitized.ends_with("Assistant:\n"));
    }

    #[test]
    fn test_calculate_hash_consistency() {
        let text = "test string";
        let hash1 = ConversationManager::calculate_hash(text);
        let hash2 = ConversationManager::calculate_hash(text);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_calculate_hash_different_inputs() {
        let hash1 = ConversationManager::calculate_hash("hello");
        let hash2 = ConversationManager::calculate_hash("world");
        assert_ne!(hash1, hash2);
    }

    #[tokio::test]
    #[spec("CHAT-014")]
    async fn test_conversation_manager_new() {
        let storage = Arc::new(MemoryStorage::new().await.unwrap());
        let manager = ConversationManager::new(storage);
        assert!(manager.is_ok());
        assert!(manager.unwrap().current_session_id().is_none());
    }

    #[tokio::test]
    #[spec("CHAT-015")]
    async fn test_start_session_with_metadata() {
        let storage = Arc::new(MemoryStorage::new().await.unwrap());
        let mut manager = ConversationManager::new(storage).unwrap();

        let session_id = manager
            .start_session_with_metadata("test-model", None, None, Some("7B".to_string()))
            .await;

        assert!(session_id.is_ok());
        assert!(manager.current_session_id().is_some());
    }

    #[tokio::test]
    #[spec("CHAT-017")]
    async fn test_add_message() {
        let storage = Arc::new(MemoryStorage::new().await.unwrap());
        let mut manager = ConversationManager::new(storage).unwrap();
        manager.start_session("test-model").await.unwrap();

        let result = manager.add_message_str("user", "Hello").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[spec("CHAT-022")]
    async fn test_get_session_stats() {
        let storage = Arc::new(MemoryStorage::new().await.unwrap());
        let mut manager = ConversationManager::new(storage).unwrap();
        manager.start_session("test-model").await.unwrap();
        manager.add_message_str("user", "Hello").await.unwrap();

        let stats = manager.get_session_stats().await;
        assert!(stats.is_ok());
        let (turn_count, token_count, fence_balanced) = stats.unwrap();
        assert_eq!(turn_count, 1);
        assert!(token_count > 0);
        assert!(fence_balanced);
    }

    #[tokio::test]
    #[spec("CHAT-023")]
    async fn test_clear_session() {
        let storage = Arc::new(MemoryStorage::new().await.unwrap());
        let mut manager = ConversationManager::new(storage).unwrap();
        manager.start_session("test-model").await.unwrap();
        assert!(manager.current_session_id().is_some());

        manager.clear_session().unwrap();
        assert!(manager.current_session_id().is_none());
    }

    #[tokio::test]
    #[spec("CHAT-016")]
    async fn test_restore_last_session_with_compatibility() {
        let storage = Arc::new(MemoryStorage::new_test().await.unwrap());
        let mut manager = ConversationManager::new(storage.clone()).unwrap();

        let compatible_id = manager
            .start_session_with_metadata(
                "test-model-v1",
                Some("chat-template-v1"),
                Some("system-prompt-v1"),
                Some("7B".to_string()),
            )
            .await
            .unwrap();

        // Restoring with matching metadata should return the session.
        let restored = manager
            .restore_last_session_with_compatibility(
                Some("chat-template-v1"),
                Some("system-prompt-v1"),
                Some("test-model-v2"),
            )
            .await
            .unwrap();
        assert_eq!(restored, Some(compatible_id));
        assert_eq!(manager.current_session_id(), Some(compatible_id));

        // Create a newer, incompatible session.
        let _incompatible_id = manager
            .start_session_with_metadata(
                "other-model-v1",
                Some("chat-template-v2"),
                Some("system-prompt-v2"),
                None,
            )
            .await
            .unwrap();

        // Restoring with the older compatible metadata should now fail
        // because the latest session is incompatible.
        let incompatible_restore = manager
            .restore_last_session_with_compatibility(
                Some("chat-template-v1"),
                Some("system-prompt-v1"),
                Some("test-model-v2"),
            )
            .await
            .unwrap();
        assert_eq!(incompatible_restore, None);
    }

    #[tokio::test]
    #[spec("CHAT-018")]
    async fn test_get_context_messages_with_limits() {
        let storage = Arc::new(MemoryStorage::new_test().await.unwrap());
        let mut manager = ConversationManager::new(storage).unwrap();
        manager.start_session("test-model").await.unwrap();

        // Seed 12 short messages alternating user/assistant.
        for i in 0..6 {
            manager
                .add_message_str("user", &format!("question {i}"))
                .await
                .unwrap();
            manager
                .add_message_str("assistant", &format!("answer {i}"))
                .await
                .unwrap();
        }

        let system = arkavo_llm::Message::system("You are helpful.");
        let context = manager
            .get_context_messages_with_limits(Some(system.clone()), Some(5))
            .await
            .unwrap();

        // System message + at most 5 history turns (10 messages).
        assert!(context.len() <= 11);
        assert_eq!(context.first().map(|m| &m.content), Some(&system.content));
        assert_eq!(
            context.first().map(|m| m.role.clone()),
            Some(arkavo_llm::Role::System)
        );
        // The newest user message should be present.
        assert!(context.iter().any(|m| m.content.contains("question 5")));

        // A smaller explicit limit should further reduce the returned history.
        let limited_context = manager
            .get_context_messages_with_limits(None, Some(2))
            .await
            .unwrap();
        // Up to 2 turns (4 messages) when no system message is supplied.
        assert!(limited_context.len() <= 4);
        assert!(
            limited_context
                .iter()
                .any(|m| m.content.contains("question 5"))
        );
    }

    #[tokio::test]
    #[spec("CHAT-019")]
    async fn test_create_summary() {
        let storage = Arc::new(MemoryStorage::new_test().await.unwrap());
        let mut manager = ConversationManager::new(storage.clone()).unwrap();
        let session_id = manager.start_session("test-model").await.unwrap();

        manager
            .add_message_str("user", "What is Rust?")
            .await
            .unwrap();
        manager
            .add_message_str("assistant", "Rust is a systems language.")
            .await
            .unwrap();

        let messages = vec![
            ConversationMessage {
                id: Uuid::new_v4(),
                session_id,
                role: "user".to_string(),
                content: "What is Rust?".to_string(),
                timestamp: Utc::now(),
                token_count: 4,
                is_summary: false,
            },
            ConversationMessage {
                id: Uuid::new_v4(),
                session_id,
                role: "assistant".to_string(),
                content: "Rust is a systems language.".to_string(),
                timestamp: Utc::now(),
                token_count: 5,
                is_summary: false,
            },
        ];

        let provider = MockSummaryProvider {
            response: "Summary of Rust discussion".to_string(),
        };
        let client = arkavo_llm::LlmClient::new(Box::new(provider));

        let summary = manager.create_summary(&client, messages).await.unwrap();
        assert_eq!(summary, "Summary of Rust discussion");

        // Verify the summary was persisted and tagged as a summary.
        let results = storage
            .search("type:conversation_summary", 10, Some("conversation"))
            .await
            .unwrap();
        let summary_messages: Vec<ConversationMessage> = results
            .into_iter()
            .filter_map(|r| serde_json::from_str::<ConversationMessage>(&r.memory.content).ok())
            .filter(|m| m.is_summary)
            .collect();
        assert!(!summary_messages.is_empty());
        assert!(
            summary_messages
                .iter()
                .any(|m| m.content.contains("Summary of Rust discussion"))
        );
    }

    #[tokio::test]
    #[spec("CHAT-020")]
    async fn test_list_sessions() {
        let storage = Arc::new(MemoryStorage::new_test().await.unwrap());
        let mut manager = ConversationManager::new(storage).unwrap();

        let id_a = manager.start_session("model-a").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let id_b = manager.start_session("model-b").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let id_c = manager.start_session("model-c").await.unwrap();

        // Semantic search can be approximate with only a few vectors; retry until
        // all created sessions are visible.
        let mut sessions = Vec::new();
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            sessions = manager.list_sessions().await.unwrap();
            let ids: std::collections::HashSet<Uuid> = sessions.iter().map(|s| s.id).collect();
            if ids.contains(&id_a) && ids.contains(&id_b) && ids.contains(&id_c) {
                break;
            }
        }

        let ids: std::collections::HashSet<Uuid> = sessions.iter().map(|s| s.id).collect();
        assert!(ids.contains(&id_a), "session a should be listed");
        assert!(ids.contains(&id_b), "session b should be listed");
        assert!(ids.contains(&id_c), "session c should be listed");

        // Sessions should be sorted by updated_at descending (newest first).
        assert_eq!(sessions[0].id, id_c);
        assert_eq!(sessions[1].id, id_b);
        assert_eq!(sessions[2].id, id_a);
    }

    #[tokio::test]
    #[spec("CHAT-021")]
    async fn test_switch_session() {
        let storage = Arc::new(MemoryStorage::new_test().await.unwrap());
        let mut manager = ConversationManager::new(storage).unwrap();

        let id_a = manager.start_session("model-a").await.unwrap();
        let id_b = manager.start_session("model-b").await.unwrap();
        assert_eq!(manager.current_session_id(), Some(id_b));

        manager.switch_session(id_a).await.unwrap();
        assert_eq!(manager.current_session_id(), Some(id_a));

        let missing = Uuid::new_v4();
        let result = manager.switch_session(missing).await;
        assert!(result.is_err());
        assert_eq!(manager.current_session_id(), Some(id_a));
    }

    #[tokio::test]
    #[spec("CHAT-014")]
    async fn test_conversation_manager_shares_storage() {
        let storage = Arc::new(MemoryStorage::new_test().await.unwrap());
        let mut manager_a = ConversationManager::new(storage.clone()).unwrap();
        let mut manager_b = ConversationManager::new(storage.clone()).unwrap();

        let session_id = manager_a.start_session("shared-model").await.unwrap();
        let restored = manager_b.restore_last_session().await.unwrap();
        assert_eq!(restored, Some(session_id));
    }

    #[tokio::test]
    #[spec("CHAT-015")]
    async fn test_start_session_metadata_persisted() {
        let storage = Arc::new(MemoryStorage::new_test().await.unwrap());
        let mut manager = ConversationManager::new(storage).unwrap();

        let session_id = manager
            .start_session_with_metadata(
                "test-model-v2",
                Some("template-a"),
                Some("prompt-a"),
                Some("7B".to_string()),
            )
            .await
            .unwrap();

        let sessions = manager.list_sessions().await.unwrap();
        let session = sessions
            .into_iter()
            .find(|s| s.id == session_id)
            .expect("session listed");
        assert_eq!(session.model, "test-model-v2");
        assert_eq!(
            session.chat_template_hash,
            Some(ConversationManager::calculate_hash("template-a"))
        );
        assert_eq!(
            session.system_prompt_hash,
            Some(ConversationManager::calculate_hash("prompt-a"))
        );
        assert_eq!(session.model_size_hint, Some("7B".to_string()));
    }

    #[tokio::test]
    #[spec("CHAT-016")]
    async fn test_restore_last_session_with_no_sessions_returns_none() {
        let storage = Arc::new(MemoryStorage::new_test().await.unwrap());
        let mut manager = ConversationManager::new(storage).unwrap();

        let restored = manager
            .restore_last_session_with_compatibility(None, None, None)
            .await
            .unwrap();
        assert_eq!(restored, None);
    }

    #[tokio::test]
    #[spec("CHAT-017")]
    async fn test_add_message_without_session_fails() {
        let storage = Arc::new(MemoryStorage::new_test().await.unwrap());
        let manager = ConversationManager::new(storage).unwrap();

        let result = manager.add_message_str("user", "Hello").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No active conversation session")
        );
    }

    #[tokio::test]
    #[spec("CHAT-018")]
    async fn test_get_context_messages_respects_token_budget() {
        let storage = Arc::new(MemoryStorage::new_test().await.unwrap());
        let mut manager = ConversationManager::new(storage).unwrap();
        manager.start_session("test-model").await.unwrap();

        // Seed many long messages that exceed the token budget.
        for _ in 0..20 {
            manager
                .add_message_str("user", &"token ".repeat(200))
                .await
                .unwrap();
        }

        let context = manager
            .get_context_messages_with_limits(None, None)
            .await
            .unwrap();
        let total_tokens: usize = context
            .iter()
            .map(|m| manager.count_tokens(&m.content))
            .sum();
        assert!(
            total_tokens <= MAX_CONTEXT_TOKENS,
            "context must stay within token budget"
        );
    }

    #[test]
    fn test_debug_flag() {
        set_debug(true);
        assert!(is_debug());
        set_debug(false);
        assert!(!is_debug());
    }
}
