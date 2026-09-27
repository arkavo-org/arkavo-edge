use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CycleId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CorrelationId(pub uuid::Uuid);

#[derive(Debug)]
pub enum MessageDisposition {
    /// Included in the current cycle's prompt
    Incorporated { cycle_id: CycleId },
    /// Queued for next cycle (current cycle was already assembling)
    Deferred,
    /// Rejected (budget exceeded, agent shutting down, etc.)
    Rejected { reason: String },
}

#[derive(Debug)]
pub struct CycleReceipt {
    pub cycle_id: CycleId,
    pub correlation_id: CorrelationId,
    pub disposition: MessageDisposition,
}

/// What the cycle that served a request actually produced.
///
/// The receipt only says the message was picked up; it says nothing about the
/// answer. Without a second signal a requester has no way to tell a finished
/// cycle from a wedged one, so it waits out its own timeout in silence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CycleOutcome {
    /// Assistant text, or a summary of the cycle's tool activity when the
    /// model answered with tool calls only.
    Completed { text: String },
    /// The cycle could not produce an answer: a routing, budget, policy or
    /// timeout refusal, or a cycle that yielded nothing at all. `error`
    /// carries the underlying message so the requester can act on it.
    Failed { error: String },
}

pub enum AgentEvent {
    IncomingMessage {
        /// did:key of the sending agent
        sender: String,
        content: String,
        task_id: uuid::Uuid,
        correlation_id: CorrelationId,
        reply: oneshot::Sender<CycleReceipt>,
        /// Answered once the serving cycle finishes, fails, or is rejected.
        outcome: oneshot::Sender<CycleOutcome>,
    },
    HumanOverride {
        instruction: String,
        correlation_id: CorrelationId,
        reply: oneshot::Sender<CycleReceipt>,
        outcome: oneshot::Sender<CycleOutcome>,
    },
    /// MCP server push notification (game state change, etc.)
    /// Fire-and-forget — no CycleReceipt reply needed.
    Notification {
        server: String,
        event_data: String,
        correlation_id: CorrelationId,
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
    /// Held until the cycle that incorporates this message finishes, so the
    /// requester is answered even when the cycle fails or is skipped.
    pub outcome: Option<oneshot::Sender<CycleOutcome>>,
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
            outcome: None,
            priority: MessagePriority::Normal,
        };
        messages.push(normal);
        let override_msg = PendingMessage {
            content: "override".into(),
            task_id: None,
            correlation_id: CorrelationId(uuid::Uuid::new_v4()),
            reply: None,
            outcome: None,
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
