use crate::dsl::{Condition, ConditionOperator, Link, Node, NodeKind, Rule, RuleType};
use crate::engine::{ControlMessage, Message};
use anyhow::Result;
use std::fmt;
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;

type OutputChannels = Arc<RwLock<Vec<(Link, mpsc::Sender<Message>)>>>;

pub struct NodeExecutor {
    node: Node,
    receiver: Arc<RwLock<mpsc::Receiver<Message>>>,
    outputs: OutputChannels,
    config: Arc<crate::DataflowConfig>,
    task_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
    metrics: Arc<RwLock<NodeMetrics>>,
}

impl Debug for NodeExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeExecutor")
            .field("node", &self.node)
            .field("metrics", &self.metrics)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct NodeMetrics {
    pub messages_received: u64,
    pub messages_sent: u64,
    pub messages_filtered: u64,
    pub messages_errored: u64,
    pub processing_time_ms: u64,
    pub queue_depth: usize,
}

impl NodeExecutor {
    pub fn new(
        node: Node,
        receiver: mpsc::Receiver<Message>,
        config: Arc<crate::DataflowConfig>,
    ) -> Result<Self> {
        Ok(Self {
            node,
            receiver: Arc::new(RwLock::new(receiver)),
            outputs: Arc::new(RwLock::new(Vec::new())),
            config,
            task_handle: Arc::new(RwLock::new(None)),
            metrics: Arc::new(RwLock::new(NodeMetrics::default())),
        })
    }

    pub fn add_output(&mut self, link: Link, sender: mpsc::Sender<Message>) {
        let outputs = self.outputs.clone();
        tokio::spawn(async move {
            let mut outputs = outputs.write().await;
            outputs.push((link, sender));
        });
    }

    pub async fn start(&self) -> Result<()> {
        let node = self.node.clone();
        let receiver = self.receiver.clone();
        let outputs = self.outputs.clone();
        let config = self.config.clone();
        let metrics = self.metrics.clone();

        let handle = tokio::spawn(async move {
            Self::run_node(node, receiver, outputs, config, metrics).await;
        });

        {
            let mut task_handle = self.task_handle.write().await;
            *task_handle = Some(handle);
        }

        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        let handle = self.task_handle.write().await.take();
        if let Some(handle) = handle {
            handle.abort();
        }
        Ok(())
    }

    pub async fn get_metrics(&self) -> NodeMetrics {
        self.metrics.read().await.clone()
    }

    async fn run_node(
        node: Node,
        receiver: Arc<RwLock<mpsc::Receiver<Message>>>,
        outputs: OutputChannels,
        _config: Arc<crate::DataflowConfig>,
        metrics: Arc<RwLock<NodeMetrics>>,
    ) {
        let mut running = false;
        #[allow(unused_assignments)]
        let _ = ();

        loop {
            let mut rx = receiver.write().await;
            match rx.recv().await {
                Some(Message::Control(ControlMessage::Start)) => {
                    running = true;
                }
                Some(Message::Control(ControlMessage::Stop)) => {
                    break;
                }
                Some(Message::Control(ControlMessage::Flush)) => {
                    // Process any remaining messages
                }
                Some(Message::Control(ControlMessage::Metrics)) => {
                    // Metrics are collected passively
                }
                Some(Message::Data {
                    payload, metadata, ..
                }) if running => {
                    let start_time = std::time::Instant::now();

                    // Update received metric
                    metrics.write().await.messages_received += 1;

                    // Process based on node type
                    let processed_message = match node.kind {
                        NodeKind::Source => {
                            // Sources generate data, so we might transform the incoming trigger
                            Some(
                                Message::new_data(payload, &node.id)
                                    .with_trace_id(metadata.trace_id),
                            )
                        }
                        NodeKind::Transform => {
                            // Apply transformation logic
                            Self::transform_message(payload, &node).map(|p| {
                                Message::new_data(p, &node.id).with_trace_id(metadata.trace_id)
                            })
                        }
                        NodeKind::Sink => {
                            // Sinks consume data, potentially storing or outputting it
                            Self::sink_message(payload, &node);
                            None
                        }
                        NodeKind::Router => {
                            // Routers pass through but may modify routing
                            Some(
                                Message::new_data(payload, &node.id)
                                    .with_trace_id(metadata.trace_id),
                            )
                        }
                    };

                    // Send to outputs based on rules
                    if let Some(msg) = processed_message {
                        let mut sent_count = 0;
                        let mut filtered_count = 0;

                        {
                            let outputs = outputs.read().await;
                            for (link, sender) in outputs.iter() {
                                if Self::should_send(&msg, link.rule.as_ref()) {
                                    if sender.send(msg.clone()).await.is_ok() {
                                        sent_count += 1;
                                    } else {
                                        metrics.write().await.messages_errored += 1;
                                    }
                                } else {
                                    filtered_count += 1;
                                }
                            }
                        }

                        let mut m = metrics.write().await;
                        m.messages_sent += sent_count;
                        m.messages_filtered += filtered_count;
                    }

                    // Update processing time
                    let elapsed = start_time.elapsed().as_millis() as u64;
                    metrics.write().await.processing_time_ms += elapsed;
                }
                Some(_) => {
                    // Message received while not running, ignore
                }
                None => {
                    // Channel closed
                    break;
                }
            }
        }
    }

    fn transform_message(payload: serde_json::Value, node: &Node) -> Option<serde_json::Value> {
        // Basic transformation logic - can be extended
        match node.params.get("transform_type") {
            Some(serde_json::Value::String(t)) if t == "uppercase" => {
                if let serde_json::Value::Object(mut map) = payload {
                    if let Some(serde_json::Value::String(s)) = map.get_mut("message") {
                        *s = s.to_uppercase();
                    }
                    Some(serde_json::Value::Object(map))
                } else {
                    Some(payload)
                }
            }
            _ => Some(payload),
        }
    }

    fn sink_message(payload: serde_json::Value, node: &Node) {
        // Basic sink logic - can be extended
        if let Some(serde_json::Value::String(sink_type)) = node.params.get("type") {
            match sink_type.as_str() {
                "console" => {
                    println!(
                        "[{}] {}",
                        node.id,
                        serde_json::to_string_pretty(&payload).unwrap_or_default()
                    );
                }
                "debug" => {
                    eprintln!("[DEBUG {}] {:?}", node.id, payload);
                }
                _ => {}
            }
        }
    }

    fn should_send(message: &Message, rule: Option<&Rule>) -> bool {
        let Some(rule) = rule else {
            return true; // No rule means always send
        };

        match &rule.rule_type {
            RuleType::Passthrough => true,
            RuleType::Filter => {
                if let Some(conditions) = &rule.conditions {
                    Self::evaluate_conditions(message, conditions)
                } else {
                    true
                }
            }
            _ => true, // Other rule types handled differently
        }
    }

    fn evaluate_conditions(message: &Message, conditions: &[Condition]) -> bool {
        let Message::Data { payload, .. } = message else {
            return false;
        };

        // All conditions must match (AND logic)
        for condition in conditions {
            if !Self::evaluate_condition(payload, condition) {
                return false;
            }
        }

        true
    }

    fn evaluate_condition(payload: &serde_json::Value, condition: &Condition) -> bool {
        // Extract field value using dot notation
        let field_value = Self::extract_field(payload, &condition.field);

        match &condition.operator {
            ConditionOperator::Contains => {
                if let (
                    Some(serde_json::Value::String(haystack)),
                    serde_json::Value::String(needle),
                ) = (field_value, &condition.value)
                {
                    haystack.contains(needle)
                } else {
                    false
                }
            }
            ConditionOperator::Equals => field_value == Some(&condition.value),
            ConditionOperator::Exists => field_value.is_some(),
            ConditionOperator::NotExists => field_value.is_none(),
            ConditionOperator::Gt => {
                Self::compare_numeric(field_value, &condition.value, |a, b| a > b)
            }
            ConditionOperator::Lt => {
                Self::compare_numeric(field_value, &condition.value, |a, b| a < b)
            }
            ConditionOperator::StartsWith => {
                if let (Some(serde_json::Value::String(text)), serde_json::Value::String(prefix)) =
                    (field_value, &condition.value)
                {
                    text.starts_with(prefix)
                } else {
                    false
                }
            }
            ConditionOperator::EndsWith => {
                if let (Some(serde_json::Value::String(text)), serde_json::Value::String(suffix)) =
                    (field_value, &condition.value)
                {
                    text.ends_with(suffix)
                } else {
                    false
                }
            }
            _ => false, // Other operators not implemented yet
        }
    }

    fn extract_field<'a>(
        value: &'a serde_json::Value,
        field_path: &str,
    ) -> Option<&'a serde_json::Value> {
        let parts: Vec<&str> = field_path.split('.').collect();
        let mut current = value;

        for part in parts {
            match current {
                serde_json::Value::Object(map) => {
                    current = map.get(part)?;
                }
                serde_json::Value::Array(arr) => {
                    let index: usize = part.parse().ok()?;
                    current = arr.get(index)?;
                }
                _ => return None,
            }
        }

        Some(current)
    }

    fn compare_numeric<F>(
        field_value: Option<&serde_json::Value>,
        compare_value: &serde_json::Value,
        compare_fn: F,
    ) -> bool
    where
        F: Fn(f64, f64) -> bool,
    {
        if let (Some(field), compare) = (field_value, compare_value) {
            match (field, compare) {
                (serde_json::Value::Number(a), serde_json::Value::Number(b)) => {
                    if let (Some(a_f64), Some(b_f64)) = (a.as_f64(), b.as_f64()) {
                        compare_fn(a_f64, b_f64)
                    } else {
                        false
                    }
                }
                _ => false,
            }
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::{Condition, ConditionOperator};

    #[tokio::test]
    async fn test_condition_evaluation() {
        let payload = serde_json::json!({
            "message": "error occurred",
            "level": "error",
            "nested": {
                "value": 42
            }
        });

        let message = Message::new_data(payload, "test");

        // Test contains
        let condition = Condition {
            field: "message".to_string(),
            operator: ConditionOperator::Contains,
            value: serde_json::json!("error"),
        };
        assert!(
            NodeExecutor::evaluate_condition(
                match &message {
                    Message::Data { payload, .. } => payload,
                    _ => panic!("Expected data message"),
                },
                &condition
            )
        );

        // Test nested field access
        let condition = Condition {
            field: "nested.value".to_string(),
            operator: ConditionOperator::Equals,
            value: serde_json::json!(42),
        };
        assert!(
            NodeExecutor::evaluate_condition(
                match &message {
                    Message::Data { payload, .. } => payload,
                    _ => panic!("Expected data message"),
                },
                &condition
            )
        );
    }
}
