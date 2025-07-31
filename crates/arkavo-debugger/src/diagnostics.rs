use arkavo_events::Event;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::health::HealthReport;

#[async_trait]
pub trait Diagnostics: Send + Sync {
    /// Get the last N events for the current agent
    async fn last_events(&self, n: usize) -> Result<Vec<Event>>;
    
    /// Get events since a specific timestamp
    async fn events_since(&self, since: DateTime<Utc>) -> Result<Vec<Event>>;
    
    /// Get the current health status
    async fn health(&self) -> Result<HealthReport>;
    
    /// Analyze recent events for error patterns
    async fn error_patterns(&self) -> Result<Vec<ErrorPattern>>;
    
    /// Get events for the current session
    async fn session_events(&self, limit: Option<usize>) -> Result<Vec<Event>>;
    
    /// Check if a specific error has occurred recently
    async fn has_recent_error(&self, error_type: &str, window_seconds: u64) -> Result<bool>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPattern {
    pub error_type: String,
    pub count: u32,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub sample_messages: Vec<String>,
    pub suggested_action: Option<String>,
}

impl ErrorPattern {
    pub fn is_recurring(&self, threshold: u32) -> bool {
        self.count >= threshold
    }
    
    pub fn frequency_per_hour(&self) -> f64 {
        let duration = self.last_seen.signed_duration_since(self.first_seen);
        let hours = duration.num_seconds() as f64 / 3600.0;
        if hours > 0.0 {
            self.count as f64 / hours
        } else {
            self.count as f64
        }
    }
}

/// Default implementation of Diagnostics that agents can use
#[cfg(feature = "vector-search")]
pub struct DefaultDiagnostics {
    agent_id: String,
    session_id: String,
    storage: arkavo_memory::storage::MemoryStorage,
}

#[cfg(feature = "vector-search")]
impl DefaultDiagnostics {
    pub fn new(
        agent_id: String,
        session_id: String,
        storage: arkavo_memory::storage::MemoryStorage,
    ) -> Self {
        Self {
            agent_id,
            session_id,
            storage,
        }
    }
}

#[cfg(feature = "vector-search")]
#[async_trait]
impl Diagnostics for DefaultDiagnostics {
    async fn last_events(&self, n: usize) -> Result<Vec<Event>> {
        let stored_events = self.storage.get_recent_events(Some(&self.agent_id), n).await?;
        
        // Convert StoredEvent to Event
        let mut events = Vec::new();
        for stored in stored_events {
            let payload = bincode::deserialize(&stored.payload)?;
            let event = Event {
                id: stored.id.parse()?,
                session_id: stored.session_id,
                sequence: stored.sequence as u64,
                timestamp: DateTime::parse_from_rfc3339(&stored.timestamp)?.with_timezone(&Utc),
                metadata: arkavo_events::EventMetadata {
                    agent_id: stored.agent_id,
                    schema_version: stored.schema_version,
                    parent_event_id: None,
                    correlation_id: None,
                },
                payload,
            };
            events.push(event);
        }
        
        Ok(events)
    }
    
    async fn events_since(&self, since: DateTime<Utc>) -> Result<Vec<Event>> {
        let stored_events = self.storage.get_events_since(since, Some(&self.agent_id)).await?;
        
        // Convert StoredEvent to Event
        let mut events = Vec::new();
        for stored in stored_events {
            let payload = bincode::deserialize(&stored.payload)?;
            let event = Event {
                id: stored.id.parse()?,
                session_id: stored.session_id,
                sequence: stored.sequence as u64,
                timestamp: DateTime::parse_from_rfc3339(&stored.timestamp)?.with_timezone(&Utc),
                metadata: arkavo_events::EventMetadata {
                    agent_id: stored.agent_id,
                    schema_version: stored.schema_version,
                    parent_event_id: None,
                    correlation_id: None,
                },
                payload,
            };
            events.push(event);
        }
        
        Ok(events)
    }
    
    async fn health(&self) -> Result<HealthReport> {
        let (session_count, event_count) = self.storage.get_event_stats().await?;
        
        Ok(HealthReport {
            status: crate::health::HealthStatus::Healthy,
            agent_id: self.agent_id.clone(),
            session_id: self.session_id.clone(),
            uptime_seconds: 0, // TODO: Track actual uptime
            total_events: event_count,
            total_sessions: session_count,
            error_rate: 0.0, // TODO: Calculate from recent errors
            last_error: None,
        })
    }
    
    async fn error_patterns(&self) -> Result<Vec<ErrorPattern>> {
        // Get recent events and analyze for patterns
        let events = self.last_events(1000).await?;
        let mut patterns: std::collections::HashMap<String, ErrorPattern> = std::collections::HashMap::new();
        
        for event in events {
            if let arkavo_events::EventPayload::Error { error_type, message, .. } = &event.payload {
                let pattern = patterns.entry(error_type.clone()).or_insert_with(|| ErrorPattern {
                    error_type: error_type.clone(),
                    count: 0,
                    first_seen: event.timestamp,
                    last_seen: event.timestamp,
                    sample_messages: Vec::new(),
                    suggested_action: None,
                });
                
                pattern.count += 1;
                pattern.last_seen = event.timestamp;
                if pattern.first_seen > event.timestamp {
                    pattern.first_seen = event.timestamp;
                }
                if pattern.sample_messages.len() < 3 && !pattern.sample_messages.contains(message) {
                    pattern.sample_messages.push(message.clone());
                }
            }
        }
        
        // Add suggested actions for known patterns
        for pattern in patterns.values_mut() {
            pattern.suggested_action = match pattern.error_type.as_str() {
                "timeout" if pattern.is_recurring(3) => Some("Consider increasing timeout or checking network connection".to_string()),
                "rate_limit" if pattern.is_recurring(5) => Some("Implement exponential backoff or reduce request frequency".to_string()),
                "authentication" => Some("Check API credentials and token expiration".to_string()),
                _ => None,
            };
        }
        
        Ok(patterns.into_values().collect())
    }
    
    async fn session_events(&self, limit: Option<usize>) -> Result<Vec<Event>> {
        let stored_events = self.storage.get_session_events(&self.session_id, limit).await?;
        
        // Convert StoredEvent to Event
        let mut events = Vec::new();
        for stored in stored_events {
            let payload = bincode::deserialize(&stored.payload)?;
            let event = Event {
                id: stored.id.parse()?,
                session_id: stored.session_id,
                sequence: stored.sequence as u64,
                timestamp: DateTime::parse_from_rfc3339(&stored.timestamp)?.with_timezone(&Utc),
                metadata: arkavo_events::EventMetadata {
                    agent_id: stored.agent_id,
                    schema_version: stored.schema_version,
                    parent_event_id: None,
                    correlation_id: None,
                },
                payload,
            };
            events.push(event);
        }
        
        Ok(events)
    }
    
    async fn has_recent_error(&self, error_type: &str, window_seconds: u64) -> Result<bool> {
        let since = Utc::now() - chrono::Duration::seconds(window_seconds as i64);
        let events = self.events_since(since).await?;
        
        Ok(events.iter().any(|e| {
            matches!(&e.payload, arkavo_events::EventPayload::Error { error_type: et, .. } if et == error_type)
        }))
    }
}