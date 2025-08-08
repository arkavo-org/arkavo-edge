use crate::error::{A2aError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushNotification {
    pub id: String,
    pub subscription_id: String,
    pub event_type: EventType,
    pub payload: serde_json::Value,
    pub timestamp: i64,
    pub retry_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    TaskUpdate,
    AgentStatus,
    MetricsUpdate,
    FileTransferProgress,
    ChatMessage,
    SystemAlert,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub client_id: String,
    pub event_types: Vec<EventType>,
    pub filter: Option<SubscriptionFilter>,
    pub created_at: i64,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionFilter {
    pub agent_ids: Option<Vec<String>>,
    pub task_ids: Option<Vec<String>>,
    pub metadata_filters: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionRequest {
    pub client_id: String,
    pub event_types: Vec<EventType>,
    pub filter: Option<SubscriptionFilter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionResponse {
    pub subscription_id: String,
    pub status: String,
}

#[derive(Clone)]
pub struct NotificationChannel {
    pub client_id: String,
    pub sender: mpsc::Sender<PushNotification>,
}

pub struct PushNotificationService {
    subscriptions: Arc<RwLock<HashMap<String, Subscription>>>,
    client_subscriptions: Arc<RwLock<HashMap<String, Vec<String>>>>,
    notification_channels: Arc<RwLock<HashMap<String, NotificationChannel>>>,
    retry_queue: Arc<RwLock<Vec<PushNotification>>>,
    max_retries: u32,
}

impl PushNotificationService {
    pub fn new(max_retries: u32) -> Self {
        Self {
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            client_subscriptions: Arc::new(RwLock::new(HashMap::new())),
            notification_channels: Arc::new(RwLock::new(HashMap::new())),
            retry_queue: Arc::new(RwLock::new(Vec::new())),
            max_retries,
        }
    }

    pub async fn subscribe(&self, request: SubscriptionRequest) -> Result<SubscriptionResponse> {
        let subscription_id = Uuid::new_v4().to_string();

        let subscription = Subscription {
            id: subscription_id.clone(),
            client_id: request.client_id.clone(),
            event_types: request.event_types,
            filter: request.filter,
            created_at: chrono::Utc::now().timestamp(),
            active: true,
        };

        let mut subscriptions = self.subscriptions.write().await;
        subscriptions.insert(subscription_id.clone(), subscription.clone());

        let mut client_subs = self.client_subscriptions.write().await;
        client_subs
            .entry(request.client_id.clone())
            .or_insert_with(Vec::new)
            .push(subscription_id.clone());

        info!(
            "Created subscription {} for client {}",
            subscription_id, request.client_id
        );

        Ok(SubscriptionResponse {
            subscription_id,
            status: "active".to_string(),
        })
    }

    pub async fn unsubscribe(&self, subscription_id: &str) -> Result<()> {
        let mut subscriptions = self.subscriptions.write().await;

        if let Some(subscription) = subscriptions.get_mut(subscription_id) {
            subscription.active = false;
            info!("Deactivated subscription {}", subscription_id);
            Ok(())
        } else {
            Err(A2aError::InvalidRequest(format!(
                "Subscription not found: {}",
                subscription_id
            )))
        }
    }

    pub async fn register_channel(
        &self,
        client_id: String,
        sender: mpsc::Sender<PushNotification>,
    ) -> Result<()> {
        let channel = NotificationChannel {
            client_id: client_id.clone(),
            sender,
        };

        let mut channels = self.notification_channels.write().await;
        channels.insert(client_id.clone(), channel);

        info!("Registered notification channel for client {}", client_id);
        Ok(())
    }

    pub async fn unregister_channel(&self, client_id: &str) -> Result<()> {
        let mut channels = self.notification_channels.write().await;

        if channels.remove(client_id).is_some() {
            info!("Unregistered notification channel for client {}", client_id);
            Ok(())
        } else {
            warn!("Channel not found for client {}", client_id);
            Err(A2aError::InvalidRequest(format!(
                "Channel not found for client: {}",
                client_id
            )))
        }
    }

    pub async fn publish_event(
        &self,
        event_type: EventType,
        payload: serde_json::Value,
        target_filter: Option<SubscriptionFilter>,
    ) -> Result<u32> {
        let subscriptions = self.subscriptions.read().await;
        let channels = self.notification_channels.read().await;

        let mut notifications_sent = 0u32;

        for subscription in subscriptions.values() {
            if !subscription.active {
                continue;
            }

            if !self.matches_event_type(&subscription.event_types, &event_type) {
                continue;
            }

            if !self.matches_filter(&subscription.filter, &target_filter) {
                continue;
            }

            let notification = PushNotification {
                id: Uuid::new_v4().to_string(),
                subscription_id: subscription.id.clone(),
                event_type: event_type.clone(),
                payload: payload.clone(),
                timestamp: chrono::Utc::now().timestamp(),
                retry_count: 0,
            };

            if let Some(channel) = channels.get(&subscription.client_id) {
                match channel.sender.try_send(notification.clone()) {
                    Ok(_) => {
                        notifications_sent += 1;
                        debug!(
                            "Sent notification to client {} for subscription {}",
                            subscription.client_id, subscription.id
                        );
                    }
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        warn!(
                            "Channel full for client {}, queueing for retry",
                            subscription.client_id
                        );
                        self.queue_for_retry(notification).await;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        warn!(
                            "Channel closed for client {}, removing",
                            subscription.client_id
                        );
                    }
                }
            } else {
                debug!(
                    "No active channel for client {}, queueing notification",
                    subscription.client_id
                );
                self.queue_for_retry(notification).await;
            }
        }

        info!(
            "Published event {:?} to {} subscribers",
            event_type, notifications_sent
        );
        Ok(notifications_sent)
    }

    async fn queue_for_retry(&self, mut notification: PushNotification) {
        if notification.retry_count >= self.max_retries {
            warn!(
                "Dropping notification {} after {} retries",
                notification.id, self.max_retries
            );
            return;
        }

        notification.retry_count += 1;
        let mut queue = self.retry_queue.write().await;
        queue.push(notification);
    }

    pub async fn process_retry_queue(&self) -> Result<u32> {
        let mut queue = self.retry_queue.write().await;
        let notifications: Vec<_> = queue.drain(..).collect();
        drop(queue);

        let channels = self.notification_channels.read().await;
        let subscriptions = self.subscriptions.read().await;

        let mut retried = 0u32;
        let mut requeue = Vec::new();

        for notification in notifications {
            if let Some(subscription) = subscriptions.get(&notification.subscription_id) {
                if let Some(channel) = channels.get(&subscription.client_id) {
                    match channel.sender.try_send(notification.clone()) {
                        Ok(_) => {
                            retried += 1;
                            debug!("Retry successful for notification {}", notification.id);
                        }
                        Err(_) => {
                            requeue.push(notification);
                        }
                    }
                } else {
                    requeue.push(notification);
                }
            }
        }

        if !requeue.is_empty() {
            let mut queue = self.retry_queue.write().await;
            for mut notification in requeue {
                if notification.retry_count < self.max_retries {
                    notification.retry_count += 1;
                    queue.push(notification);
                }
            }
        }

        Ok(retried)
    }

    fn matches_event_type(&self, subscription_types: &[EventType], event_type: &EventType) -> bool {
        subscription_types.iter().any(|t| {
            matches!(
                (t, event_type),
                (EventType::Custom(a), EventType::Custom(b)) if a == b
            ) || std::mem::discriminant(t) == std::mem::discriminant(event_type)
        })
    }

    fn matches_filter(
        &self,
        subscription_filter: &Option<SubscriptionFilter>,
        target_filter: &Option<SubscriptionFilter>,
    ) -> bool {
        match (subscription_filter, target_filter) {
            (None, _) => true,
            (Some(_), None) => true,
            (Some(sub_filter), Some(target)) => {
                if let (Some(sub_agents), Some(target_agents)) =
                    (&sub_filter.agent_ids, &target.agent_ids)
                {
                    if !sub_agents.iter().any(|a| target_agents.contains(a)) {
                        return false;
                    }
                }

                if let (Some(sub_tasks), Some(target_tasks)) =
                    (&sub_filter.task_ids, &target.task_ids)
                {
                    if !sub_tasks.iter().any(|t| target_tasks.contains(t)) {
                        return false;
                    }
                }

                true
            }
        }
    }

    pub async fn get_client_subscriptions(&self, client_id: &str) -> Vec<Subscription> {
        let subscriptions = self.subscriptions.read().await;
        let client_subs = self.client_subscriptions.read().await;

        if let Some(sub_ids) = client_subs.get(client_id) {
            sub_ids
                .iter()
                .filter_map(|id| subscriptions.get(id).cloned())
                .filter(|sub| sub.active) // Only return active subscriptions
                .collect()
        } else {
            Vec::new()
        }
    }

    pub async fn cleanup_inactive_subscriptions(&self) -> Result<usize> {
        let mut subscriptions = self.subscriptions.write().await;
        let mut client_subs = self.client_subscriptions.write().await;

        let before_count = subscriptions.len();

        subscriptions.retain(|_, sub| sub.active);

        for subs in client_subs.values_mut() {
            subs.retain(|id| subscriptions.contains_key(id));
        }

        client_subs.retain(|_, subs| !subs.is_empty());

        let removed = before_count - subscriptions.len();
        if removed > 0 {
            info!("Cleaned up {} inactive subscriptions", removed);
        }

        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_subscription_lifecycle() {
        let service = PushNotificationService::new(3);

        let request = SubscriptionRequest {
            client_id: "client1".to_string(),
            event_types: vec![EventType::TaskUpdate, EventType::ChatMessage],
            filter: None,
        };

        let response = service.subscribe(request).await.unwrap();
        assert!(!response.subscription_id.is_empty());

        let subs = service.get_client_subscriptions("client1").await;
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].id, response.subscription_id);

        service
            .unsubscribe(&response.subscription_id)
            .await
            .unwrap();

        let cleaned = service.cleanup_inactive_subscriptions().await.unwrap();
        assert_eq!(cleaned, 1);

        let subs = service.get_client_subscriptions("client1").await;
        assert_eq!(subs.len(), 0);
    }

    #[tokio::test]
    async fn test_event_publishing() {
        let service = PushNotificationService::new(3);

        let (tx, mut rx) = mpsc::channel(10);
        service
            .register_channel("client1".to_string(), tx)
            .await
            .unwrap();

        let request = SubscriptionRequest {
            client_id: "client1".to_string(),
            event_types: vec![EventType::TaskUpdate],
            filter: None,
        };

        service.subscribe(request).await.unwrap();

        let payload = serde_json::json!({
            "task_id": "task123",
            "status": "completed"
        });

        let sent = service
            .publish_event(EventType::TaskUpdate, payload.clone(), None)
            .await
            .unwrap();

        assert_eq!(sent, 1);

        let notification = rx.recv().await.unwrap();
        assert_eq!(notification.payload, payload);
    }

    #[tokio::test]
    async fn test_event_filtering() {
        let service = PushNotificationService::new(3);

        let filter = SubscriptionFilter {
            agent_ids: Some(vec!["agent1".to_string()]),
            task_ids: None,
            metadata_filters: None,
        };

        let request = SubscriptionRequest {
            client_id: "client1".to_string(),
            event_types: vec![EventType::TaskUpdate],
            filter: Some(filter),
        };

        service.subscribe(request).await.unwrap();

        let target_filter = SubscriptionFilter {
            agent_ids: Some(vec!["agent2".to_string()]),
            task_ids: None,
            metadata_filters: None,
        };

        let sent = service
            .publish_event(
                EventType::TaskUpdate,
                serde_json::json!({}),
                Some(target_filter),
            )
            .await
            .unwrap();

        assert_eq!(sent, 0);
    }
}
