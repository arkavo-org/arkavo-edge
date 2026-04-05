use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CycleId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CorrelationId(pub uuid::Uuid);

#[derive(Debug)]
pub enum MessageDisposition {
    Incorporated { cycle_id: CycleId },
    Deferred,
    Rejected { reason: String },
}

#[derive(Debug)]
pub struct CycleReceipt {
    pub cycle_id: CycleId,
    pub correlation_id: CorrelationId,
    pub disposition: MessageDisposition,
}

pub enum AgentEvent {
    IncomingMessage {
        sender: String,
        content: String,
        task_id: uuid::Uuid,
        correlation_id: CorrelationId,
        reply: oneshot::Sender<CycleReceipt>,
    },
    HumanOverride {
        instruction: String,
        correlation_id: CorrelationId,
        reply: oneshot::Sender<CycleReceipt>,
    },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessagePriority {
    Normal,
    Override,
}

pub struct PendingMessage {
    pub content: String,
    pub task_id: Option<uuid::Uuid>,
    pub correlation_id: CorrelationId,
    pub reply: Option<oneshot::Sender<CycleReceipt>>,
    pub priority: MessagePriority,
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("SRV-010")]
    #[test]
    fn test_cycle_id_is_copy() {
        let id = CycleId(42);
        let copy = id;
        assert_eq!(id, copy);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_correlation_id_is_copy() {
        let id = CorrelationId(uuid::Uuid::new_v4());
        let copy = id;
        assert_eq!(id, copy);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_pending_message_priority_ordering() {
        let mut messages: Vec<PendingMessage> = Vec::new();
        let normal = PendingMessage {
            content: "normal".into(),
            task_id: None,
            correlation_id: CorrelationId(uuid::Uuid::new_v4()),
            reply: None,
            priority: MessagePriority::Normal,
        };
        messages.push(normal);
        let override_msg = PendingMessage {
            content: "override".into(),
            task_id: None,
            correlation_id: CorrelationId(uuid::Uuid::new_v4()),
            reply: None,
            priority: MessagePriority::Override,
        };
        messages.insert(0, override_msg);
        assert_eq!(messages[0].priority, MessagePriority::Override);
        assert_eq!(messages[1].priority, MessagePriority::Normal);
    }

    #[spec("SRV-010")]
    #[tokio::test]
    async fn test_cycle_receipt_flows_through_oneshot() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let correlation_id = CorrelationId(uuid::Uuid::new_v4());
        let receipt = CycleReceipt {
            cycle_id: CycleId(5),
            correlation_id,
            disposition: MessageDisposition::Incorporated {
                cycle_id: CycleId(5),
            },
        };
        tx.send(receipt).unwrap();
        let received = rx.await.unwrap();
        assert_eq!(received.cycle_id, CycleId(5));
        assert_eq!(received.correlation_id, correlation_id);
        assert!(matches!(
            received.disposition,
            MessageDisposition::Incorporated { .. }
        ));
    }

    #[spec("SRV-010")]
    #[tokio::test]
    async fn test_dropped_sender_returns_error() {
        let (tx, rx) = tokio::sync::oneshot::channel::<CycleReceipt>();
        drop(tx);
        assert!(rx.await.is_err());
    }
}
