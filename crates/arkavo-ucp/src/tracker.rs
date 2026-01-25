use crate::error::{Result, UcpError};
use crate::types::{PaymentIntent, PaymentResult, PaymentStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Record of a payment for tracking and auditing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentRecord {
    pub intent: PaymentIntent,
    pub result: Option<PaymentResult>,
    pub history: Vec<PaymentStatusChange>,
}

impl PaymentRecord {
    pub fn new(intent: PaymentIntent) -> Self {
        let initial_status = PaymentStatusChange {
            from: None,
            to: intent.status,
            timestamp: intent.created_at,
            reason: Some("Payment intent created".to_string()),
        };

        Self {
            intent,
            result: None,
            history: vec![initial_status],
        }
    }

    pub fn status(&self) -> PaymentStatus {
        self.result
            .as_ref()
            .map(|r| r.status)
            .unwrap_or(self.intent.status)
    }

    pub fn update_status(&mut self, status: PaymentStatus, reason: Option<String>) {
        let change = PaymentStatusChange {
            from: Some(self.status()),
            to: status,
            timestamp: Utc::now(),
            reason,
        };
        self.history.push(change);
        self.intent.status = status;
    }

    pub fn complete(&mut self, result: PaymentResult) {
        self.update_status(result.status, result.error.clone());
        self.result = Some(result);
    }
}

/// Status change event for audit trail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentStatusChange {
    pub from: Option<PaymentStatus>,
    pub to: PaymentStatus,
    pub timestamp: DateTime<Utc>,
    pub reason: Option<String>,
}

/// Payment tracker for managing payment lifecycle
pub struct PaymentTracker {
    payments: RwLock<HashMap<Uuid, PaymentRecord>>,
    by_agent: RwLock<HashMap<String, Vec<Uuid>>>,
}

impl PaymentTracker {
    pub fn new() -> Self {
        Self {
            payments: RwLock::new(HashMap::new()),
            by_agent: RwLock::new(HashMap::new()),
        }
    }

    /// Create and track a new payment intent
    pub async fn create(&self, intent: PaymentIntent) -> Uuid {
        let id = intent.id;
        let agent_id = intent.agent_id.clone();
        let record = PaymentRecord::new(intent);

        let mut payments = self.payments.write().await;
        payments.insert(id, record);

        let mut by_agent = self.by_agent.write().await;
        by_agent.entry(agent_id).or_default().push(id);

        id
    }

    /// Get a payment by ID
    pub async fn get(&self, id: Uuid) -> Option<PaymentRecord> {
        let payments = self.payments.read().await;
        payments.get(&id).cloned()
    }

    /// Get all payments for an agent
    pub async fn get_by_agent(&self, agent_id: &str) -> Vec<PaymentRecord> {
        let by_agent = self.by_agent.read().await;
        let payments = self.payments.read().await;

        by_agent
            .get(agent_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| payments.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Update payment status
    pub async fn update_status(
        &self,
        id: Uuid,
        status: PaymentStatus,
        reason: Option<String>,
    ) -> Result<()> {
        let mut payments = self.payments.write().await;
        let record = payments
            .get_mut(&id)
            .ok_or_else(|| UcpError::PaymentNotFound(id.to_string()))?;

        record.update_status(status, reason);
        Ok(())
    }

    /// Complete a payment with result
    pub async fn complete(&self, id: Uuid, result: PaymentResult) -> Result<()> {
        let mut payments = self.payments.write().await;
        let record = payments
            .get_mut(&id)
            .ok_or_else(|| UcpError::PaymentNotFound(id.to_string()))?;

        record.complete(result);
        Ok(())
    }

    /// Get pending payments (for retry/cleanup)
    pub async fn get_pending(&self) -> Vec<PaymentRecord> {
        let payments = self.payments.read().await;
        payments
            .values()
            .filter(|r| {
                matches!(
                    r.status(),
                    PaymentStatus::Pending | PaymentStatus::Processing
                )
            })
            .cloned()
            .collect()
    }

    /// Get expired payments
    pub async fn get_expired(&self) -> Vec<PaymentRecord> {
        let payments = self.payments.read().await;
        payments
            .values()
            .filter(|r| r.intent.is_expired() && !r.status().is_terminal())
            .cloned()
            .collect()
    }

    /// Clean up expired payments
    pub async fn expire_stale(&self) -> Vec<Uuid> {
        let expired = self.get_expired().await;
        let mut expired_ids = Vec::new();

        for record in expired {
            let id = record.intent.id;
            if self
                .update_status(
                    id,
                    PaymentStatus::Expired,
                    Some("Payment expired".to_string()),
                )
                .await
                .is_ok()
            {
                expired_ids.push(id);
            }
        }

        expired_ids
    }

    /// Get payment statistics for an agent
    pub async fn get_stats(&self, agent_id: &str) -> PaymentStats {
        let payments = self.get_by_agent(agent_id).await;

        let mut stats = PaymentStats::default();
        for record in payments {
            match record.status() {
                PaymentStatus::Completed => {
                    stats.completed += 1;
                    stats.total_spent += record.intent.amount.value;
                }
                PaymentStatus::Failed => stats.failed += 1,
                PaymentStatus::Pending | PaymentStatus::Processing => stats.pending += 1,
                _ => {}
            }
        }

        stats
    }
}

impl Default for PaymentTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Payment statistics for an agent
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PaymentStats {
    pub pending: u32,
    pub completed: u32,
    pub failed: u32,
    pub total_spent: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Merchant, PaymentAmount};

    fn create_test_intent(agent_id: &str) -> PaymentIntent {
        let merchant = Merchant::new("test", "Test Merchant");
        let amount = PaymentAmount::from_dollars(10.0);
        PaymentIntent::new(agent_id, amount, merchant)
    }

    #[tokio::test]
    async fn test_create_and_get() {
        let tracker = PaymentTracker::new();
        let intent = create_test_intent("agent-1");
        let id = intent.id;

        tracker.create(intent).await;

        let record = tracker.get(id).await;
        assert!(record.is_some());
        assert_eq!(record.unwrap().intent.id, id);
    }

    #[tokio::test]
    async fn test_get_by_agent() {
        let tracker = PaymentTracker::new();

        let intent1 = create_test_intent("agent-1");
        let intent2 = create_test_intent("agent-1");
        let intent3 = create_test_intent("agent-2");

        tracker.create(intent1).await;
        tracker.create(intent2).await;
        tracker.create(intent3).await;

        let agent1_payments = tracker.get_by_agent("agent-1").await;
        let agent2_payments = tracker.get_by_agent("agent-2").await;

        assert_eq!(agent1_payments.len(), 2);
        assert_eq!(agent2_payments.len(), 1);
    }

    #[tokio::test]
    async fn test_update_status() {
        let tracker = PaymentTracker::new();
        let intent = create_test_intent("agent-1");
        let id = intent.id;

        tracker.create(intent).await;
        tracker
            .update_status(id, PaymentStatus::Processing, None)
            .await
            .unwrap();

        let record = tracker.get(id).await.unwrap();
        assert_eq!(record.status(), PaymentStatus::Processing);
        assert_eq!(record.history.len(), 2);
    }

    #[tokio::test]
    async fn test_complete_payment() {
        let tracker = PaymentTracker::new();
        let intent = create_test_intent("agent-1");
        let id = intent.id;

        tracker.create(intent).await;

        let result = PaymentResult::success(id);
        tracker.complete(id, result).await.unwrap();

        let record = tracker.get(id).await.unwrap();
        assert_eq!(record.status(), PaymentStatus::Completed);
        assert!(record.result.is_some());
    }

    #[tokio::test]
    async fn test_get_pending() {
        let tracker = PaymentTracker::new();

        let intent1 = create_test_intent("agent-1");
        let intent2 = create_test_intent("agent-1");
        let id2 = intent2.id;

        tracker.create(intent1).await;
        tracker.create(intent2).await;
        tracker
            .complete(id2, PaymentResult::success(id2))
            .await
            .unwrap();

        let pending = tracker.get_pending().await;
        assert_eq!(pending.len(), 1);
    }

    #[tokio::test]
    async fn test_payment_stats() {
        let tracker = PaymentTracker::new();

        let intent1 = create_test_intent("agent-1");
        let intent2 = create_test_intent("agent-1");
        let id1 = intent1.id;
        let id2 = intent2.id;

        tracker.create(intent1).await;
        tracker.create(intent2).await;

        tracker
            .complete(id1, PaymentResult::success(id1))
            .await
            .unwrap();
        tracker
            .complete(id2, PaymentResult::failed(id2, "Error"))
            .await
            .unwrap();

        let stats = tracker.get_stats("agent-1").await;
        assert_eq!(stats.completed, 1);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.pending, 0);
    }

    #[tokio::test]
    async fn test_payment_record_history() {
        let tracker = PaymentTracker::new();
        let intent = create_test_intent("agent-1");
        let id = intent.id;

        tracker.create(intent).await;
        tracker
            .update_status(id, PaymentStatus::Processing, Some("Started".to_string()))
            .await
            .unwrap();
        tracker
            .update_status(id, PaymentStatus::Signed, Some("Signed".to_string()))
            .await
            .unwrap();

        let record = tracker.get(id).await.unwrap();
        assert_eq!(record.history.len(), 3);
        assert_eq!(record.history[0].to, PaymentStatus::Pending);
        assert_eq!(record.history[1].to, PaymentStatus::Processing);
        assert_eq!(record.history[2].to, PaymentStatus::Signed);
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let tracker = PaymentTracker::new();
        let result = tracker.get(Uuid::new_v4()).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_update_nonexistent() {
        let tracker = PaymentTracker::new();
        let result = tracker
            .update_status(Uuid::new_v4(), PaymentStatus::Completed, None)
            .await;
        assert!(result.is_err());
    }
}
