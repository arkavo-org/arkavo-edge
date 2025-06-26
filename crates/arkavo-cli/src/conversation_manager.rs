use arkavo_llm::{LlmClient, Message};
use arkavo_memory::storage::MemoryStorage;
use chrono::{DateTime, Utc};
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tiktoken_rs::{CoreBPE, cl100k_base};
use uuid::Uuid;

const MAX_CONTEXT_MESSAGES: usize = 10; // Conservative default for Ollama
const MAX_CONTEXT_TOKENS: usize = 1500; // Conservative default for Ollama
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
}

pub struct ConversationManager {
    memory_storage: Arc<MemoryStorage>,
    token_encoder: CoreBPE,
    current_session_id: Option<Uuid>,
}

impl ConversationManager {
    pub async fn new(memory_storage: Arc<MemoryStorage>) -> anyhow::Result<Self> {
        Ok(Self {
            memory_storage,
            token_encoder: cl100k_base()?,
            current_session_id: None,
        })
    }

    pub async fn start_session(&mut self, model: &str) -> anyhow::Result<Uuid> {
        let session_id = Uuid::new_v4();
        let session = ConversationSession {
            id: session_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            model: model.to_string(),
            title: None,
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

        if let Some(latest_session) = sessions.first() {
            if let Ok(session) =
                serde_json::from_str::<ConversationSession>(&latest_session.memory.content)
            {
                self.current_session_id = Some(session.id);
                progress.finish_with_message(format!(
                    "Restored session from {}",
                    session.created_at.format("%Y-%m-%d %H:%M")
                ));
                return Ok(Some(session.id));
            }
        }

        progress.finish_and_clear();
        Ok(None)
    }

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

    pub async fn get_context_messages(
        &self,
        system_message: Option<Message>,
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
        let query = format!("session_id:{}", session_id);
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
            let summary_query = format!("session_id:{} type:summary", session_id);
            let summaries = self
                .memory_storage
                .search(&summary_query, 5, Some("conversation"))
                .await?;

            if let Some(latest_summary) = summaries.first() {
                // Use the summary as context
                if let Ok(summary_msg) =
                    serde_json::from_str::<ConversationMessage>(&latest_summary.memory.content)
                {
                    context_messages.push(Message::assistant(&summary_msg.content));
                }
            }
        }

        // Add recent messages within token budget
        let mut total_tokens = self.count_message_tokens(&context_messages);
        let recent_messages: Vec<_> = messages
            .iter()
            .rev()
            .take(MAX_CONTEXT_MESSAGES)
            .rev()
            .collect();

        for msg in recent_messages {
            let msg_tokens = msg.token_count;
            if total_tokens + msg_tokens > MAX_CONTEXT_TOKENS {
                break;
            }

            let message = match msg.role.as_str() {
                "user" => Message::user(&msg.content),
                "assistant" => Message::assistant(&msg.content),
                "system" => Message::system(&msg.content),
                _ => continue,
            };

            context_messages.push(message);
            total_tokens += msg_tokens;
        }

        progress.finish_and_clear();
        Ok(context_messages)
    }

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
            summary_prompt.push_str(&format!("{}: {}\n\n", msg.role.to_uppercase(), msg.content));
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
            content: format!("Previous conversation summary:\n{}", summary),
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
        let query = format!("session_id:{}", session_id);
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

    pub fn get_current_session_id(&self) -> Option<Uuid> {
        self.current_session_id
    }

    fn count_tokens(&self, text: &str) -> usize {
        self.token_encoder.encode_with_special_tokens(text).len()
    }

    fn count_message_tokens(&self, messages: &[Message]) -> usize {
        messages
            .iter()
            .map(|msg| self.count_tokens(&msg.content))
            .sum()
    }
}
