//! `arp/get` — expose this agent's effective ARP document over A2A.
//!
//! Each agent in the mesh owns its own ARP. The gateway polls this method
//! per agent so the AG-UI audit views can show real per-agent documents
//! instead of the synthetic `(local)` placeholder. The document returned is
//! the proposal queue's post-apply document, so applied tightenings are
//! visible to mesh-wide audit.

use std::sync::Arc;

use arkavo_arp_runtime::ArpRuntime;
use jsonrpsee::core::RpcResult;
use serde_json::{Value, json};

/// Build the `arp/get` payload from an optional runtime. `document` is null
/// when no ARP runtime is installed in this process.
pub(in crate::server) async fn arp_get_payload(runtime: Option<Arc<ArpRuntime>>) -> Value {
    let document = match runtime {
        Some(rt) => {
            let queue = rt.proposal_queue();
            let guard = queue.lock().await;
            serde_json::to_value(guard.document()).unwrap_or(Value::Null)
        }
        None => Value::Null,
    };
    json!({
        "document": document,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    })
}

/// Handle the `arp/get` A2A method.
pub(in crate::server) async fn handle_arp_get() -> RpcResult<Value> {
    Ok(arp_get_payload(arkavo_arp_runtime::current()).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn no_runtime_yields_null_document() {
        let payload = arp_get_payload(None).await;
        assert!(payload["document"].is_null());
        assert!(payload["timestamp"].is_string());
    }

    #[tokio::test]
    async fn runtime_yields_post_apply_document() {
        const DOC: &str = r#"{
            "arp_spec": "0.1.0",
            "adl_ref": {"uri": "https://example.com/adl.json"},
            "adaptation": {"method": "thompson_sampling"},
            "feedback_loops": {
                "immediate": {
                    "quality_gate": {
                        "threshold_default": 0.7,
                        "metric": "cosine_similarity",
                        "on_failure": "update_prior_and_log"
                    }
                },
                "short_term": {
                    "policy_cache": {
                        "default_ttl_sec": 3600,
                        "decay_strategy": "linear"
                    }
                }
            },
            "budget": {
                "task_ceiling_usd": 2.5,
                "on_exhaustion": "halt_and_report",
                "velocity": {"max_spend_per_minute_usd": 0.5}
            }
        }"#;
        let doc = arkavo_arp::parse(DOC).unwrap();
        let rt = Arc::new(ArpRuntime::from_document(&doc));
        let payload = arp_get_payload(Some(rt)).await;
        assert_eq!(payload["document"]["arp_spec"], "0.1.0");
        assert_eq!(payload["document"]["budget"]["task_ceiling_usd"], 2.5);
    }
}
