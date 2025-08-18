use arkavo_llm::LlmClient;
use arkavo_memory::storage::MemoryStorage;
use chrono::{DateTime, Utc};
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tiktoken_rs::{CoreBPE, cl100k_base};
use uuid::Uuid;

#[allow(dead_code)]
const MAX_CONTEXT_MESSAGES: usize = 10; // Conservative default for Ollama
#[allow(dead_code)]
const MAX_CONTEXT_TOKENS: usize = 1500; // Conservative default for Ollama
#[allow(dead_code)]
const SUMMARY_TRIGGER_MESSAGES: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub id: Uuid,
    pub session_id: Uuid,
    pub role: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub token_count: usize,
    pub is_summary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSession {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub model: String,
    pub title: Option<String>,
    pub chat_template_hash: Option<String>,
    pub system_prompt_hash: Option<String>,
    pub model_size_hint: Option<String>, // e.g., "270M", "1B", "7B"
}

pub struct ConversationManager {
    memory_storage: Arc<MemoryStorage>,
    #[allow(dead_code)]
    token_encoder: CoreBPE,
    current_session_id: Option<Uuid>,
}

impl ConversationManager {
    pub fn new(memory_storage: Arc<MemoryStorage>) -> anyhow::Result<Self> {
        Ok(Self {
            memory_storage,
            token_encoder: cl100k_base()?,
            current_session_id: None,
        })
    }

    /// Sanitize message content to prevent generation issues with small models
    fn sanitize_message_content(content: &str) -> String {
        let mut sanitized = content.to_string();

        // Count and balance triple backticks
        let fence_count = sanitized.matches("```").count();
        if fence_count % 2 != 0 {
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

    pub async fn start_session(&mut self, model: &str) -> anyhow::Result<Uuid> {
        self.start_session_with_metadata(model, None, None, None)
            .await
    }

    pub async fn start_session_with_metadata(
        &mut self,
        model: &str,
        chat_template: Option<&str>,
        system_prompt: Option<&str>,
        model_size: Option<&str>,
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
            model_size_hint: model_size.map(|s| s.to_string()),
        };

        // Store session metadata
        let memory = arkavo_memory::models::Memory {
            id: Uuid::new_v4(),
            content: serde_json::to_string(&session)?,
            metadata: Some(json!({
                "type": "conversation_session",
                "session_id": session_id,
                "model": model,
            })),
            category: Some("conversation".to_string()),
            embedding: vec![0.0; 384], // Placeholder
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.memory_storage.store(memory).await?;
        self.current_session_id = Some(session_id);
        Ok(session_id)
    }

    pub async fn restore_last_session(&mut self) -> anyhow::Result<Option<Uuid>> {
        self.restore_last_session_with_compatibility(None, None, None)
            .await
    }

    #[allow(clippy::missing_panics_doc)]
    #[allow(clippy::literal_string_with_formatting_args)]
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
                // Extract base model name (remove quantization suffix)
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
            // Session incompatible
            incompatibility_reason = incompatibility_reason.trim_end_matches(", ").to_string();
            progress.finish_with_message(format!(
                "Session skipped: {incompatibility_reason} (starting fresh)"
            ));
            return Ok(None);
        }

        progress.finish_and_clear();
        Ok(None)
    }

    pub async fn add_message(&self, message: &arkavo_llm::Message) -> anyhow::Result<()> {
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

        // Store the message
        let memory = arkavo_memory::models::Memory {
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
            embedding: vec![0.0; 384], // Placeholder
            created_at: conv_message.timestamp,
            updated_at: conv_message.timestamp,
        };

        self.memory_storage.store(memory).await?;
        Ok(())
    }

    #[allow(
        clippy::missing_panics_doc,
        clippy::literal_string_with_formatting_args
    )]
    pub async fn get_context_messages(
        &self,
        system_message: Option<arkavo_llm::Message>,
    ) -> anyhow::Result<Vec<arkavo_llm::Message>> {
        self.get_context_messages_with_limits(system_message, None)
            .await
    }

    /// Get context messages with optional limits
    ///
    /// # Panics
    ///
    /// May panic if progress style template is invalid
    #[allow(clippy::literal_string_with_formatting_args)]
    pub async fn get_context_messages_with_limits(
        &self,
        system_message: Option<arkavo_llm::Message>,
        max_history_turns: Option<usize>,
    ) -> anyhow::Result<Vec<arkavo_llm::Message>> {
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
        messages.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

        // Apply sliding window
        let mut context_messages = Vec::new();
        if let Some(sys_msg) = system_message {
            context_messages.push(sys_msg);
        }

        // Check if we need summarization
        if messages.len() > SUMMARY_TRIGGER_MESSAGES {
            progress.set_message("Checking for conversation summary...");

            // Look for existing summary
            let summary_query = format!("session_id:{session_id} type:summary");
            let summaries = self
                .memory_storage
                .search(&summary_query, 5, Some("conversation"))
                .await?;

            if let Some(latest_summary) = summaries.first() {
                // Use the summary as context
                if let Ok(summary_msg) =
                    serde_json::from_str::<ConversationMessage>(&latest_summary.memory.content)
                {
                    context_messages.push(arkavo_llm::Message::assistant(&summary_msg.content));
                }
            }
        }

        // Determine effective history limit
        let history_limit = if let Some(limit) = max_history_turns {
            limit
        } else {
            // Check environment variable or use default based on model size
            std::env::var("ARKAVO_MAX_HISTORY_TURNS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(MAX_CONTEXT_MESSAGES)
        };

        // Add recent messages within token budget and history limit
        let mut total_tokens = self.count_message_tokens(&context_messages);
        let recent_messages: Vec<_> = messages.iter().rev().take(history_limit).rev().collect();

        for (idx, msg) in recent_messages.iter().enumerate() {
            let msg_tokens = msg.token_count;
            if total_tokens + msg_tokens > MAX_CONTEXT_TOKENS {
                break;
            }

            // Sanitize content - especially important for the last message
            let sanitized_content = if idx == recent_messages.len() - 1 {
                Self::sanitize_message_content(&msg.content)
            } else {
                msg.content.clone()
            };

            let message = match msg.role.as_str() {
                "user" => arkavo_llm::Message::user(&sanitized_content),
                "assistant" => arkavo_llm::Message::assistant(&sanitized_content),
                "system" => arkavo_llm::Message::system(&sanitized_content),
                _ => continue,
            };

            context_messages.push(message);
            total_tokens += msg_tokens;
        }

        // Debug output using tracing
        tracing::debug!(
            "Context messages: {} messages, {} tokens",
            context_messages.len(),
            total_tokens
        );

        // Check fence parity in final context for debugging
        let full_context = context_messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let fence_count = full_context.matches("```").count();
        if fence_count % 2 != 0 {
            tracing::warn!("Fence parity: {} (UNBALANCED!)", fence_count);
        } else {
            tracing::debug!("Fence parity: {} (balanced)", fence_count);
        }

        progress.finish_and_clear();
        Ok(context_messages)
    }

    #[allow(clippy::missing_panics_doc)]
    #[allow(clippy::literal_string_with_formatting_args)]
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

        // Build prompt for summarization
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
            arkavo_llm::Message::system(
                "You are a helpful assistant that creates concise conversation summaries.",
            ),
            arkavo_llm::Message::user(&summary_prompt),
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

        let memory = arkavo_memory::models::Memory {
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

    pub fn clear_session(&mut self) -> anyhow::Result<()> {
        self.current_session_id = None;
        Ok(())
    }

    pub async fn get_session_stats(&self) -> anyhow::Result<(usize, usize, bool)> {
        let session_id = self
            .current_session_id
            .ok_or_else(|| anyhow::anyhow!("No active conversation session"))?;

        // Query messages for this session
        let query = format!("session_id:{session_id}");
        let results = self
            .memory_storage
            .search(&query, 100, Some("conversation"))
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

        // Check fence parity
        let full_text = messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let fence_count = full_text.matches("```").count();
        let fence_balanced = fence_count % 2 == 0;

        Ok((turn_count, token_count, fence_balanced))
    }

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

        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }

    pub async fn switch_session(&mut self, session_id: Uuid) -> anyhow::Result<()> {
        // Verify session exists
        let query = format!("session_id:{session_id}");
        let results = self
            .memory_storage
            .search(&query, 1, Some("conversation"))
            .await?;

        if results.is_empty() {
            return Err(anyhow::anyhow!("Session not found"));
        }

        self.current_session_id = Some(session_id);
        Ok(())
    }

    pub const fn get_current_session_id(&self) -> Option<Uuid> {
        self.current_session_id
    }

    #[allow(dead_code)]
    fn count_tokens(&self, text: &str) -> usize {
        self.token_encoder.encode_with_special_tokens(text).len()
    }

    fn count_message_tokens(&self, messages: &[arkavo_llm::Message]) -> usize {
        messages
            .iter()
            .map(|msg| self.count_tokens(&msg.content))
            .sum()
    }
}
