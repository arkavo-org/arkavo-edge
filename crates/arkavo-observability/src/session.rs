use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{error, info, instrument, warn};

/// Session state enum for tracking lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    /// Session is active and processing messages
    Active,
    /// Session is being closed gracefully
    Closing,
    /// Session is in zombie state (cleanup needed)
    Zombie,
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionState::Active => write!(f, "active"),
            SessionState::Closing => write!(f, "closing"),
            SessionState::Zombie => write!(f, "zombie"),
        }
    }
}

/// Session metrics and observability data
#[derive(Debug, Clone)]
pub struct SessionMetrics {
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub state: SessionState,
    pub messages_sent: Arc<AtomicU64>,
    pub messages_received: Arc<AtomicU64>,
    pub deltas_sent: Arc<AtomicU64>,
    pub errors_count: Arc<AtomicU64>,
    pub last_activity: Arc<std::sync::Mutex<DateTime<Utc>>>,
}

impl SessionMetrics {
    /// Create new session metrics
    #[instrument(skip(session_id), fields(session.id = %session_id))]
    pub fn new(session_id: String) -> Self {
        let now = Utc::now();
        info!("Session metrics initialized");

        Self {
            session_id,
            created_at: now,
            state: SessionState::Active,
            messages_sent: Arc::new(AtomicU64::new(0)),
            messages_received: Arc::new(AtomicU64::new(0)),
            deltas_sent: Arc::new(AtomicU64::new(0)),
            errors_count: Arc::new(AtomicU64::new(0)),
            last_activity: Arc::new(std::sync::Mutex::new(now)),
        }
    }

    /// Record a message sent to the session
    #[instrument(skip(self), fields(session.id = %self.session_id))]
    pub fn record_message_sent(&self, message_length: usize) {
        let count = self.messages_sent.fetch_add(1, Ordering::Relaxed) + 1;
        self.update_activity();

        info!(
            message.count = count,
            message.length = message_length,
            "Message sent recorded"
        );
    }

    /// Record a message received by the session
    #[instrument(skip(self), fields(session.id = %self.session_id))]
    pub fn record_message_received(&self) {
        let count = self.messages_received.fetch_add(1, Ordering::Relaxed) + 1;
        self.update_activity();

        info!(message.count = count, "Message received recorded");
    }

    /// Record a delta sent from the session
    #[instrument(skip(self), fields(session.id = %self.session_id))]
    pub fn record_delta_sent(&self, delta_type: &str, sequence: u64) {
        let count = self.deltas_sent.fetch_add(1, Ordering::Relaxed) + 1;
        self.update_activity();

        info!(
            delta.count = count,
            delta.type = delta_type,
            delta.sequence = sequence,
            "Delta sent recorded"
        );
    }

    /// Record an error in the session
    #[instrument(skip(self), fields(session.id = %self.session_id))]
    pub fn record_error(&self, error_type: &str, error_message: &str) {
        let count = self.errors_count.fetch_add(1, Ordering::Relaxed) + 1;
        self.update_activity();

        error!(
            error.count = count,
            error.type = error_type,
            error.message = error_message,
            "Session error recorded"
        );
    }

    /// Update session state
    #[instrument(skip(self), fields(session.id = %self.session_id))]
    pub fn set_state(&mut self, new_state: SessionState) {
        let old_state = self.state;
        self.state = new_state;

        info!(
            state.old = %old_state,
            state.new = %new_state,
            "Session state changed"
        );
    }

    /// Check if session is stale (no activity for given duration)
    pub fn is_stale(&self, ttl_seconds: u64) -> bool {
        if let Ok(last_activity) = self.last_activity.lock() {
            let now = Utc::now();
            let duration = now.signed_duration_since(*last_activity);
            duration.num_seconds() as u64 > ttl_seconds
        } else {
            false
        }
    }

    /// Get session duration
    pub fn session_duration(&self) -> chrono::Duration {
        Utc::now().signed_duration_since(self.created_at)
    }

    /// Get current metrics snapshot
    pub fn snapshot(&self) -> SessionMetricsSnapshot {
        SessionMetricsSnapshot {
            session_id: self.session_id.clone(),
            created_at: self.created_at,
            state: self.state,
            messages_sent: self.messages_sent.load(Ordering::Relaxed),
            messages_received: self.messages_received.load(Ordering::Relaxed),
            deltas_sent: self.deltas_sent.load(Ordering::Relaxed),
            errors_count: self.errors_count.load(Ordering::Relaxed),
            last_activity: *self.last_activity.lock().unwrap(),
            session_duration_ms: self.session_duration().num_milliseconds() as u64,
        }
    }

    fn update_activity(&self) {
        if let Ok(mut last_activity) = self.last_activity.lock() {
            *last_activity = Utc::now();
        }
    }
}

/// Immutable snapshot of session metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetricsSnapshot {
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub state: SessionState,
    pub messages_sent: u64,
    pub messages_received: u64,
    pub deltas_sent: u64,
    pub errors_count: u64,
    pub last_activity: DateTime<Utc>,
    pub session_duration_ms: u64,
}

/// TTL cleaner for managing session lifecycle
pub struct SessionTtlCleaner {
    ttl_seconds: u64,
    check_interval_seconds: u64,
}

impl SessionTtlCleaner {
    /// Create a new TTL cleaner
    pub fn new(ttl_seconds: u64, check_interval_seconds: u64) -> Self {
        Self {
            ttl_seconds,
            check_interval_seconds,
        }
    }

    /// Start the TTL cleaner task
    pub async fn start<F, Fut>(
        self,
        sessions: Arc<tokio::sync::RwLock<HashMap<String, SessionMetrics>>>,
        cleanup_callback: F,
    ) -> tokio::task::JoinHandle<()>
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        info!(
            ttl_seconds = self.ttl_seconds,
            check_interval_seconds = self.check_interval_seconds,
            "Starting session TTL cleaner"
        );

        let cleanup_callback = Arc::new(cleanup_callback);

        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(self.check_interval_seconds));

            loop {
                interval.tick().await;

                let stale_sessions = {
                    let sessions_read = sessions.read().await;
                    sessions_read
                        .values()
                        .filter(|metrics| {
                            metrics.is_stale(self.ttl_seconds)
                                || metrics.state == SessionState::Zombie
                        })
                        .map(|metrics| metrics.session_id.clone())
                        .collect::<Vec<_>>()
                };

                if !stale_sessions.is_empty() {
                    info!(
                        stale_sessions_count = stale_sessions.len(),
                        "Found stale sessions for cleanup"
                    );

                    for session_id in stale_sessions {
                        // Remove from tracking
                        {
                            let mut sessions_write = sessions.write().await;
                            if let Some(metrics) = sessions_write.remove(&session_id) {
                                info!(
                                    session.id = %session_id,
                                    session.state = %metrics.state,
                                    session.duration_ms = metrics.session_duration().num_milliseconds(),
                                    "Cleaning up stale session"
                                );
                            }
                        }

                        // Call cleanup callback
                        cleanup_callback(session_id).await;
                    }
                }
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Duration;
    use tokio::time::sleep;

    #[test]
    fn test_session_metrics() {
        let metrics = SessionMetrics::new("test-session".to_string());

        assert_eq!(metrics.state, SessionState::Active);
        assert_eq!(metrics.messages_sent.load(Ordering::Relaxed), 0);

        metrics.record_message_sent(100);
        assert_eq!(metrics.messages_sent.load(Ordering::Relaxed), 1);

        metrics.record_delta_sent("text", 1);
        assert_eq!(metrics.deltas_sent.load(Ordering::Relaxed), 1);

        metrics.record_error("test_error", "test message");
        assert_eq!(metrics.errors_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_session_state_transitions() {
        let mut metrics = SessionMetrics::new("test-session".to_string());

        assert_eq!(metrics.state, SessionState::Active);

        metrics.set_state(SessionState::Closing);
        assert_eq!(metrics.state, SessionState::Closing);

        metrics.set_state(SessionState::Zombie);
        assert_eq!(metrics.state, SessionState::Zombie);
    }

    #[test]
    fn test_session_staleness() {
        let metrics = SessionMetrics::new("test-session".to_string());

        // Fresh session should not be stale
        assert!(!metrics.is_stale(60));

        // Mock old last activity by manually setting it
        {
            let mut last_activity = metrics.last_activity.lock().unwrap();
            *last_activity = Utc::now() - chrono::Duration::seconds(120);
        }

        // Should be stale with 60 second TTL
        assert!(metrics.is_stale(60));
    }

    #[test]
    fn test_metrics_snapshot() {
        let metrics = SessionMetrics::new("test-session".to_string());
        metrics.record_message_sent(100);
        metrics.record_delta_sent("text", 1);

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.session_id, "test-session");
        assert_eq!(snapshot.messages_sent, 1);
        assert_eq!(snapshot.deltas_sent, 1);
        assert_eq!(snapshot.state, SessionState::Active);
    }

    #[tokio::test]
    async fn test_ttl_cleaner() {
        let sessions = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        let cleaned_sessions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let cleaned_sessions_clone = cleaned_sessions.clone();

        // Add a session that will be stale
        {
            let metrics = SessionMetrics::new("stale-session".to_string());
            // Set last activity to 2 minutes ago
            {
                let mut last_activity = metrics.last_activity.lock().unwrap();
                *last_activity = Utc::now() - chrono::Duration::seconds(120);
            }
            sessions
                .write()
                .await
                .insert("stale-session".to_string(), metrics);
        }

        let cleaner = SessionTtlCleaner::new(60, 1); // 60s TTL, check every 1s

        let cleanup_handle = cleaner
            .start(sessions.clone(), move |session_id| {
                let cleaned = cleaned_sessions_clone.clone();
                async move {
                    cleaned.lock().unwrap().push(session_id);
                }
            })
            .await;

        // Wait for cleanup to happen
        sleep(Duration::from_secs(2)).await;

        // Check that session was cleaned up
        assert!(sessions.read().await.is_empty());
        assert_eq!(cleaned_sessions.lock().unwrap().len(), 1);
        assert_eq!(cleaned_sessions.lock().unwrap()[0], "stale-session");

        cleanup_handle.abort();
    }

    #[tokio::test]
    async fn test_ttl_cleaner_with_zombie_sessions() {
        let sessions = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        let cleaned_sessions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let cleaned_sessions_clone = cleaned_sessions.clone();

        // Add a zombie session
        {
            let mut metrics = SessionMetrics::new("zombie-session".to_string());
            metrics.set_state(SessionState::Zombie);
            sessions
                .write()
                .await
                .insert("zombie-session".to_string(), metrics);
        }

        // Add a fresh session that should not be cleaned
        {
            let metrics = SessionMetrics::new("fresh-session".to_string());
            sessions
                .write()
                .await
                .insert("fresh-session".to_string(), metrics);
        }

        let cleaner = SessionTtlCleaner::new(60, 1); // 60s TTL, check every 1s

        let cleanup_handle = cleaner
            .start(sessions.clone(), move |session_id| {
                let cleaned = cleaned_sessions_clone.clone();
                async move {
                    cleaned.lock().unwrap().push(session_id);
                }
            })
            .await;

        // Wait for cleanup to happen
        sleep(Duration::from_secs(2)).await;

        // Check that only zombie session was cleaned up
        let remaining_sessions = sessions.read().await;
        assert_eq!(remaining_sessions.len(), 1);
        assert!(remaining_sessions.contains_key("fresh-session"));

        assert_eq!(cleaned_sessions.lock().unwrap().len(), 1);
        assert_eq!(cleaned_sessions.lock().unwrap()[0], "zombie-session");

        cleanup_handle.abort();
    }

    #[tokio::test]
    async fn test_ttl_cleaner_no_cleanup_needed() {
        let sessions = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        let cleaned_sessions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let cleaned_sessions_clone = cleaned_sessions.clone();

        // Add only fresh sessions
        {
            let metrics1 = SessionMetrics::new("fresh-session-1".to_string());
            let metrics2 = SessionMetrics::new("fresh-session-2".to_string());
            sessions
                .write()
                .await
                .insert("fresh-session-1".to_string(), metrics1);
            sessions
                .write()
                .await
                .insert("fresh-session-2".to_string(), metrics2);
        }

        let cleaner = SessionTtlCleaner::new(60, 1); // 60s TTL, check every 1s

        let cleanup_handle = cleaner
            .start(sessions.clone(), move |session_id| {
                let cleaned = cleaned_sessions_clone.clone();
                async move {
                    cleaned.lock().unwrap().push(session_id);
                }
            })
            .await;

        // Wait for several cleanup cycles
        sleep(Duration::from_secs(2)).await;

        // Check that no sessions were cleaned up
        let remaining_sessions = sessions.read().await;
        assert_eq!(remaining_sessions.len(), 2);
        assert!(remaining_sessions.contains_key("fresh-session-1"));
        assert!(remaining_sessions.contains_key("fresh-session-2"));

        assert_eq!(cleaned_sessions.lock().unwrap().len(), 0);

        cleanup_handle.abort();
    }

    #[tokio::test]
    async fn test_ttl_cleaner_multiple_stale_sessions() {
        let sessions = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        let cleaned_sessions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let cleaned_sessions_clone = cleaned_sessions.clone();

        // Add multiple stale sessions
        for i in 1..=3 {
            let metrics = SessionMetrics::new(format!("stale-session-{i}"));
            // Set last activity to 2 minutes ago
            {
                let mut last_activity = metrics.last_activity.lock().unwrap();
                *last_activity = Utc::now() - chrono::Duration::seconds(120);
            }
            sessions
                .write()
                .await
                .insert(format!("stale-session-{i}"), metrics);
        }

        let cleaner = SessionTtlCleaner::new(60, 1); // 60s TTL, check every 1s

        let cleanup_handle = cleaner
            .start(sessions.clone(), move |session_id| {
                let cleaned = cleaned_sessions_clone.clone();
                async move {
                    cleaned.lock().unwrap().push(session_id);
                }
            })
            .await;

        // Wait for cleanup to happen
        sleep(Duration::from_secs(2)).await;

        // Check that all stale sessions were cleaned up
        assert!(sessions.read().await.is_empty());

        let cleaned = cleaned_sessions.lock().unwrap();
        assert_eq!(cleaned.len(), 3);
        assert!(cleaned.contains(&"stale-session-1".to_string()));
        assert!(cleaned.contains(&"stale-session-2".to_string()));
        assert!(cleaned.contains(&"stale-session-3".to_string()));

        cleanup_handle.abort();
    }

    #[test]
    fn test_ttl_cleaner_configuration() {
        let cleaner = SessionTtlCleaner::new(300, 30); // 5 minute TTL, check every 30s
        assert_eq!(cleaner.ttl_seconds, 300);
        assert_eq!(cleaner.check_interval_seconds, 30);
    }

    #[test]
    fn test_session_metrics_staleness_detection() {
        let metrics = SessionMetrics::new("test-session".to_string());

        // Fresh session should not be stale
        assert!(!metrics.is_stale(60));

        // Manually set old last activity
        {
            let mut last_activity = metrics.last_activity.lock().unwrap();
            *last_activity = Utc::now() - chrono::Duration::seconds(120);
        }

        // Should be stale with 60 second TTL
        assert!(metrics.is_stale(60));

        // Should not be stale with 200 second TTL
        assert!(!metrics.is_stale(200));
    }
}
