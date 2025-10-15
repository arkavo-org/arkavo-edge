use arkavo_events::{Event, EventWriter};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Information about an active agent session
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub endpoint: String,
    pub capabilities: Vec<String>,
    pub event_count: u64,
    pub last_activity: chrono::DateTime<chrono::Utc>,
}

/// Manages active agent sessions for debugging
pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<String, SessionInfo>>>,
    event_writers: Arc<RwLock<HashMap<String, Arc<EventWriter>>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            event_writers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new active session
    pub async fn register_session(
        &self,
        session_id: String,
        agent_id: String,
        agent_name: String,
        endpoint: String,
        capabilities: Vec<String>,
        event_writer: Option<Arc<EventWriter>>,
    ) -> Result<(), crate::error::DebugError> {
        let session_info = SessionInfo {
            session_id: session_id.clone(),
            agent_id,
            agent_name,
            start_time: chrono::Utc::now(),
            endpoint,
            capabilities,
            event_count: 0,
            last_activity: chrono::Utc::now(),
        };

        self.sessions
            .write()
            .await
            .insert(session_id.clone(), session_info);

        if let Some(writer) = event_writer {
            self.event_writers.write().await.insert(session_id, writer);
        }

        Ok(())
    }

    /// Unregister a session when it ends
    pub async fn unregister_session(&self, session_id: &str) {
        self.sessions.write().await.remove(session_id);
        self.event_writers.write().await.remove(session_id);
    }

    /// Get all active sessions
    pub async fn get_active_sessions(&self) -> Vec<SessionInfo> {
        self.sessions.read().await.values().cloned().collect()
    }

    /// Get a specific session
    pub async fn get_session(&self, session_id: &str) -> Option<SessionInfo> {
        self.sessions.read().await.get(session_id).cloned()
    }

    /// Update session activity
    pub async fn update_activity(&self, session_id: &str) {
        if let Some(session) = self.sessions.write().await.get_mut(session_id) {
            session.last_activity = chrono::Utc::now();
            session.event_count += 1;
        }
    }

    /// Get event writer for a session
    pub async fn get_event_writer(&self, session_id: &str) -> Option<Arc<EventWriter>> {
        self.event_writers.read().await.get(session_id).cloned()
    }

    /// Write an event to a specific session
    pub async fn write_event(&self, event: Event) -> Result<(), crate::error::DebugError> {
        let session_id = &event.session_id;

        // Update activity
        self.update_activity(session_id).await;

        // Write to session's event writer if available
        if let Some(writer) = self.get_event_writer(session_id).await {
            writer
                .write(event)
                .await
                .map_err(crate::error::DebugError::Event)?;
        }

        Ok(())
    }

    /// Clean up inactive sessions
    pub async fn cleanup_inactive_sessions(&self, max_age: chrono::Duration) {
        let now = chrono::Utc::now();
        let mut sessions = self.sessions.write().await;
        let mut writers = self.event_writers.write().await;

        sessions.retain(|session_id, info| {
            let age = now - info.last_activity;
            if age > max_age {
                writers.remove(session_id);
                false
            } else {
                true
            }
        });
    }

    /// Discover sessions via network scan
    pub async fn discover_sessions(&self) -> Result<Vec<SessionInfo>, crate::error::DebugError> {
        // Return active sessions for now - full network discovery would require
        // mDNS/Zeroconf integration which is available but needs additional setup
        let mut discovered = self.get_active_sessions().await;

        // Check for any sessions advertised via environment variables or known ports
        // This is a simplified implementation - in production, we'd use mDNS
        if let Ok(agent_endpoint) = std::env::var("ARKAVO_AGENT_ENDPOINT") {
            // Parse endpoint and check if reachable
            if let Ok(url) = url::Url::parse(&agent_endpoint)
                && let Some(host) = url.host_str()
            {
                // Create a placeholder session for discovered agent
                let session_info = SessionInfo {
                    session_id: format!("discovered-{}", uuid::Uuid::new_v4()),
                    agent_id: format!("agent-{}", host),
                    agent_name: format!("Discovered Agent at {}", host),
                    endpoint: agent_endpoint,
                    capabilities: vec!["discovered".to_string()],
                    start_time: chrono::Utc::now(),
                    event_count: 0,
                    last_activity: chrono::Utc::now(),
                };
                discovered.push(session_info);
            }
        }

        Ok(discovered)
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_registration() {
        let manager = SessionManager::new();

        let result = manager
            .register_session(
                "test-session".to_string(),
                "agent-1".to_string(),
                "TestAgent".to_string(),
                "localhost:8080".to_string(),
                vec!["capability1".to_string()],
                None,
            )
            .await;

        assert!(result.is_ok());

        let sessions = manager.get_active_sessions().await;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "test-session");
    }

    #[tokio::test]
    async fn test_session_cleanup() {
        let manager = SessionManager::new();

        manager
            .register_session(
                "old-session".to_string(),
                "agent-1".to_string(),
                "OldAgent".to_string(),
                "localhost:8080".to_string(),
                vec![],
                None,
            )
            .await
            .unwrap();

        // Immediately clean up with 0 duration should remove it
        manager
            .cleanup_inactive_sessions(chrono::Duration::zero())
            .await;

        let sessions = manager.get_active_sessions().await;
        assert_eq!(sessions.len(), 0);
    }
}
