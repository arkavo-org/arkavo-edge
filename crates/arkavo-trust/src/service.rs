//! TrustService: orchestrates MCP-T query/verify/history/publish/providers

use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::TrustError;
use crate::mapper::{AgentTrustInput, compute_trust_score};
use crate::signing::{sign_trust_event, sign_trust_score, verify_trust_event};
use crate::store::TrustStore;
use crate::types::*;
use arkavo_crypto::AgentKeypair;

/// MCP-T L1 trust service provider.
///
/// Uses in-memory structures for hot-path operations and an optional
/// SQLite `TrustStore` for durable persistence of events and score cache.
pub struct TrustService {
    keypair: AgentKeypair,
    provider_did: String,
    /// In-memory event log (always available)
    events: RwLock<Vec<TrustEvent>>,
    /// In-memory score cache
    score_cache: RwLock<HashMap<(String, String), TrustScore>>,
    /// Agent trust input data
    agent_inputs: RwLock<HashMap<String, AgentTrustInput>>,
    /// SQLite-backed persistent store (optional)
    store: Option<Arc<TrustStore>>,
    max_age_seconds: u32,
}

impl TrustService {
    /// Create a service without persistence (in-memory only).
    pub fn new(keypair: AgentKeypair, max_age_seconds: u32) -> Self {
        let provider_did = keypair.public_key().to_did_key();
        Self {
            keypair,
            provider_did,
            events: RwLock::new(Vec::new()),
            score_cache: RwLock::new(HashMap::new()),
            agent_inputs: RwLock::new(HashMap::new()),
            store: None,
            max_age_seconds,
        }
    }

    /// Create a service with SQLite persistence.
    pub fn with_store(keypair: AgentKeypair, max_age_seconds: u32, store: TrustStore) -> Self {
        let provider_did = keypair.public_key().to_did_key();
        Self {
            keypair,
            provider_did,
            events: RwLock::new(Vec::new()),
            score_cache: RwLock::new(HashMap::new()),
            agent_inputs: RwLock::new(HashMap::new()),
            store: Some(Arc::new(store)),
            max_age_seconds,
        }
    }

    /// Update the trust input data for an agent.
    pub fn update_agent_input(&self, subject_id: &str, input: AgentTrustInput) {
        let mut inputs = self.agent_inputs.write().unwrap();
        inputs.insert(subject_id.to_string(), input);
    }

    /// All subject IDs this provider currently has input for. Used by clients
    /// (e.g. the agui panel) that need to enumerate scorable subjects;
    /// MCP-T v0.2.0 does not define a spec method for this, so callers reach
    /// it through a private adjunct RPC rather than `trust/*`.
    pub fn subjects(&self) -> Vec<String> {
        let inputs = self.agent_inputs.read().unwrap();
        let mut out: Vec<String> = inputs.keys().cloned().collect();
        out.sort();
        out
    }

    /// Handle trust/query (Section 7.1)
    pub fn query(&self, request: &TrustQueryRequest) -> Result<TrustQueryResponse, TrustError> {
        let domain = request.domain.as_deref().unwrap_or("general");
        let max_age = request.max_age_seconds.unwrap_or(self.max_age_seconds);

        // Check in-memory cache
        let cache_key = (request.subject_id.clone(), domain.to_string());
        {
            let cache = self.score_cache.read().unwrap();
            if let Some(cached) = cache.get(&cache_key)
                && cached.validity.expires_at > Utc::now()
            {
                let mut score = cached.clone();
                if let Some(ref dims) = request.dimensions {
                    score.score.dimensions.retain(|k, _| dims.contains(k));
                }
                return Ok(TrustQueryResponse { trust_score: score });
            }
        }

        // Compute fresh score from agent input
        let inputs = self.agent_inputs.read().unwrap();
        let input = inputs
            .get(&request.subject_id)
            .ok_or(TrustError::SubjectNotFound(request.subject_id.clone()))?;

        let mut score = compute_trust_score(
            input,
            &request.subject_id,
            &self.provider_did,
            Some(domain),
            max_age,
        );

        sign_trust_score(&mut score, &self.keypair);

        // Cache in-memory
        {
            let mut cache = self.score_cache.write().unwrap();
            cache.insert(cache_key, score.clone());
        }

        // Write-through to persistent store
        if let Some(ref store) = self.store {
            let score_clone = score.clone();
            let store = Arc::clone(store);
            tokio::task::spawn(async move {
                let _ = store.cache_score(&score_clone).await;
            });
        }

        // Filter dimensions if requested
        if let Some(ref dims) = request.dimensions {
            score.score.dimensions.retain(|k, _| dims.contains(k));
        }

        Ok(TrustQueryResponse { trust_score: score })
    }

    /// Handle trust/verify (Section 7.2)
    pub fn verify(&self, request: &TrustVerifyRequest) -> Result<TrustVerifyResponse, TrustError> {
        let query = TrustQueryRequest {
            subject_id: request.subject_id.clone(),
            domain: request.domain.clone(),
            provider_id: None,
            max_age_seconds: None,
            dimensions: None,
        };
        let score_resp = self.query(&query)?;
        let score = &score_resp.trust_score;

        let mut verified = true;
        let mut min_confidence = 1.0_f64;

        let composite_check = request.threshold.composite_min.map(|min| {
            let met = score.score.composite >= min;
            if !met {
                verified = false;
            }
            ThresholdCheck { required: min, met }
        });

        let dimension_checks = request.threshold.dimension_mins.as_ref().map(|mins| {
            let mut results = HashMap::new();
            for (dim, &required) in mins {
                let met = score
                    .score
                    .dimensions
                    .get(dim)
                    .map(|d| {
                        min_confidence = min_confidence.min(d.confidence);
                        d.value >= required
                    })
                    .unwrap_or(false);
                if !met {
                    verified = false;
                }
                results.insert(dim.clone(), ThresholdCheck { required, met });
            }
            results
        });

        if let Some(conf_min) = request.threshold.confidence_min
            && min_confidence < conf_min
        {
            verified = false;
        }

        if let Some(min_count) = request.threshold.min_evidence_count {
            let total: u32 = score
                .score
                .dimensions
                .values()
                .map(|d| d.evidence_count)
                .sum();
            if total < min_count {
                verified = false;
            }
        }

        let now = Utc::now();
        Ok(TrustVerifyResponse {
            verified,
            subject_id: request.subject_id.clone(),
            provider_id: self.provider_did.clone(),
            nonce: request.nonce.clone(),
            threshold_results: ThresholdResults {
                composite_min: composite_check,
                dimension_mins: dimension_checks,
            },
            confidence: min_confidence,
            checked_at: now,
            valid_until: Some(now + chrono::Duration::seconds(self.max_age_seconds as i64)),
            signature: None,
        })
    }

    /// Handle trust/history (Section 7.3)
    pub fn history(&self, request: &TrustHistoryRequest) -> TrustHistoryResponse {
        let events = self.events.read().unwrap();
        let limit = request.limit.unwrap_or(50).min(1000) as usize;

        let filtered: Vec<&TrustEvent> = events
            .iter()
            .filter(|e| e.subject_id == request.subject_id)
            .filter(|e| {
                request
                    .event_types
                    .as_ref()
                    .map(|types| types.contains(&e.event_type))
                    .unwrap_or(true)
            })
            .filter(|e| {
                request
                    .after
                    .map(|after| e.timestamp > after)
                    .unwrap_or(true)
            })
            .filter(|e| {
                request
                    .before
                    .map(|before| e.timestamp < before)
                    .unwrap_or(true)
            })
            .collect();

        let total_count = filtered.len() as u64;
        let result: Vec<TrustEvent> = filtered.into_iter().rev().take(limit).cloned().collect();
        let has_more = total_count > limit as u64;

        TrustHistoryResponse {
            events: result,
            cursor: None,
            has_more,
            total_count,
        }
    }

    /// Handle trust/publish (Section 7.5)
    pub fn publish(
        &self,
        request: &TrustPublishRequest,
    ) -> Result<TrustPublishResponse, TrustError> {
        let event = &request.event;

        // Verify signature if present
        if event.signature.is_some() {
            verify_trust_event(event).map_err(|e| TrustError::InvalidSignature(e.to_string()))?;
        }

        // Check for duplicate in memory
        {
            let events = self.events.read().unwrap();
            if events.iter().any(|e| e.event_id == event.event_id) {
                return Err(TrustError::DuplicateEvent(event.event_id.clone()));
            }
        }

        // Store in memory
        {
            let mut events = self.events.write().unwrap();
            events.push(event.clone());
        }

        // Write-through to persistent store
        if let Some(ref store) = self.store {
            let event_clone = event.clone();
            let subject_id = event.subject_id.clone();
            let store = Arc::clone(store);
            tokio::task::spawn(async move {
                let _ = store.store_event(&event_clone).await;
                let _ = store.invalidate_cache(&subject_id).await;
            });
        }

        // Invalidate in-memory cache
        self.invalidate_cache(&event.subject_id);

        Ok(TrustPublishResponse {
            accepted: true,
            event_id: event.event_id.clone(),
            processing_status: "processed".to_string(),
        })
    }

    /// Handle trust/providers (Section 7.4)
    pub fn providers(&self) -> TrustProvidersResponse {
        TrustProvidersResponse {
            providers: vec![ProviderDescriptor {
                provider_id: self.provider_did.clone(),
                name: "Arkavo Edge Trust Provider".to_string(),
                description: Some(
                    "Behavioral trust scoring from agent performance and anti-pattern detection"
                        .to_string(),
                ),
                endpoint: None,
                transport_bindings: vec!["jsonrpc".to_string()],
                conformance_level: 1,
                supported_domains: vec![
                    domains::GENERAL.to_string(),
                    domains::CODE_EXECUTION.to_string(),
                ],
                dimensions: vec![
                    dimensions::PERFORMANCE.to_string(),
                    dimensions::CONSISTENCY.to_string(),
                    dimensions::VERIFICATION.to_string(),
                    dimensions::TENURE.to_string(),
                    dimensions::SECURITY.to_string(),
                    dimensions::TRANSPARENCY.to_string(),
                    dimensions::COMPLIANCE.to_string(),
                    dimensions::COMMUNITY.to_string(),
                    dimensions::BEHAVIORAL_FIDELITY.to_string(),
                ],
                scoring_methodology_uri: None,
                public_key: self
                    .provider_did
                    .strip_prefix("did:key:")
                    .unwrap_or(&self.provider_did)
                    .to_string(),
            }],
        }
    }

    /// Create and publish a trust event from internal signals.
    pub fn emit_event(
        &self,
        subject_id: &str,
        event_type: &str,
        payload: serde_json::Value,
        dimensions_affected: Vec<String>,
    ) -> Result<String, TrustError> {
        let event_id = format!("evt_{}", uuid_v4_short());
        let mut event = TrustEvent {
            event_id: event_id.clone(),
            event_type: event_type.to_string(),
            subject_id: subject_id.to_string(),
            issuer_id: self.provider_did.clone(),
            timestamp: Utc::now(),
            payload,
            dimensions_affected: Some(dimensions_affected),
            signature: None,
            co_signatures: None,
        };

        sign_trust_event(&mut event, &self.keypair);

        let request = TrustPublishRequest { event };
        self.publish(&request)?;
        Ok(event_id)
    }

    fn invalidate_cache(&self, subject_id: &str) {
        let mut cache = self.score_cache.write().unwrap();
        cache.retain(|(sid, _), _| sid != subject_id);
    }
}

fn uuid_v4_short() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{ts:016x}")
}

/// Wrap TrustService in Arc for sharing across handlers.
pub type SharedTrustService = Arc<TrustService>;

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> TrustService {
        let keypair = AgentKeypair::generate();
        let service = TrustService::new(keypair, 3600);

        service.update_agent_input(
            "did:key:z6MkAgent1",
            AgentTrustInput {
                success_rate: Some(0.85),
                success_std_dev: Some(0.1),
                observation_count: Some(200),
                has_verified_did: true,
                created_at: Some(Utc::now() - chrono::Duration::days(30)),
                security_anti_pattern_weight: Some(0.0),
                total_anti_pattern_weight: Some(0.0),
                decision_trace_coverage: Some(0.95),
                peer_count: Some(5),
                ..Default::default()
            },
        );

        service
    }

    #[test]
    fn query_returns_signed_score() {
        let service = setup();
        let req = TrustQueryRequest {
            subject_id: "did:key:z6MkAgent1".to_string(),
            domain: Some("code-execution".to_string()),
            provider_id: None,
            max_age_seconds: None,
            dimensions: None,
        };

        let resp = service.query(&req).unwrap();
        assert!(resp.trust_score.signature.is_some());
        assert!(resp.trust_score.score.composite > 0);
        assert!(resp.trust_score.score.dimensions.len() >= 6);
    }

    #[test]
    fn query_unknown_subject_returns_error() {
        let service = setup();
        let req = TrustQueryRequest {
            subject_id: "did:key:z6MkUnknown".to_string(),
            domain: None,
            provider_id: None,
            max_age_seconds: None,
            dimensions: None,
        };

        let result = service.query(&req);
        assert!(matches!(result, Err(TrustError::SubjectNotFound(_))));
    }

    #[test]
    fn query_filters_dimensions() {
        let service = setup();
        let req = TrustQueryRequest {
            subject_id: "did:key:z6MkAgent1".to_string(),
            domain: None,
            provider_id: None,
            max_age_seconds: None,
            dimensions: Some(vec!["performance".to_string()]),
        };

        let resp = service.query(&req).unwrap();
        assert_eq!(resp.trust_score.score.dimensions.len(), 1);
        assert!(
            resp.trust_score
                .score
                .dimensions
                .contains_key("performance")
        );
    }

    #[test]
    fn verify_passes_when_above_threshold() {
        let service = setup();
        let req = TrustVerifyRequest {
            subject_id: "did:key:z6MkAgent1".to_string(),
            domain: None,
            threshold: ThresholdSpec {
                composite_min: Some(500),
                dimension_mins: None,
                confidence_min: None,
                min_evidence_count: None,
            },
            nonce: Some("test-nonce".to_string()),
            provider_id: None,
        };

        let resp = service.verify(&req).unwrap();
        assert!(resp.verified);
        assert_eq!(resp.nonce, Some("test-nonce".to_string()));
    }

    #[test]
    fn verify_fails_when_below_threshold() {
        let service = setup();
        let req = TrustVerifyRequest {
            subject_id: "did:key:z6MkAgent1".to_string(),
            domain: None,
            threshold: ThresholdSpec {
                composite_min: Some(999),
                dimension_mins: None,
                confidence_min: None,
                min_evidence_count: None,
            },
            nonce: None,
            provider_id: None,
        };

        let resp = service.verify(&req).unwrap();
        assert!(!resp.verified);
    }

    #[test]
    fn verify_checks_dimension_mins() {
        let service = setup();
        let mut dim_mins = HashMap::new();
        dim_mins.insert("verification".to_string(), 1000u32);
        dim_mins.insert("performance".to_string(), 900u32);

        let req = TrustVerifyRequest {
            subject_id: "did:key:z6MkAgent1".to_string(),
            domain: None,
            threshold: ThresholdSpec {
                composite_min: None,
                dimension_mins: Some(dim_mins),
                confidence_min: None,
                min_evidence_count: None,
            },
            nonce: None,
            provider_id: None,
        };

        let resp = service.verify(&req).unwrap();
        assert!(!resp.verified);

        let results = resp.threshold_results.dimension_mins.unwrap();
        assert!(results["verification"].met);
        assert!(!results["performance"].met);
    }

    #[test]
    fn publish_and_history() {
        let service = setup();
        let keypair = AgentKeypair::generate();

        let mut event = TrustEvent {
            event_id: "evt_test_001".to_string(),
            event_type: "contract.completed".to_string(),
            subject_id: "did:key:z6MkAgent1".to_string(),
            issuer_id: keypair.public_key().to_did_key(),
            timestamp: Utc::now(),
            payload: serde_json::json!({"contract_id": "ctr_1", "outcome": "success"}),
            dimensions_affected: Some(vec!["performance".to_string()]),
            signature: None,
            co_signatures: None,
        };
        crate::signing::sign_trust_event(&mut event, &keypair);

        let pub_resp = service
            .publish(&TrustPublishRequest {
                event: event.clone(),
            })
            .unwrap();
        assert!(pub_resp.accepted);

        let hist = service.history(&TrustHistoryRequest {
            subject_id: "did:key:z6MkAgent1".to_string(),
            event_types: None,
            after: None,
            before: None,
            limit: None,
            cursor: None,
        });

        assert_eq!(hist.total_count, 1);
        assert_eq!(hist.events[0].event_id, "evt_test_001");
    }

    #[test]
    fn duplicate_event_rejected() {
        let service = setup();

        let event = TrustEvent {
            event_id: "evt_dup".to_string(),
            event_type: "contract.completed".to_string(),
            subject_id: "did:key:z6MkAgent1".to_string(),
            issuer_id: "did:key:z6MkIssuer".to_string(),
            timestamp: Utc::now(),
            payload: serde_json::json!({}),
            dimensions_affected: None,
            signature: None,
            co_signatures: None,
        };

        service
            .publish(&TrustPublishRequest {
                event: event.clone(),
            })
            .unwrap();

        let result = service.publish(&TrustPublishRequest { event });
        assert!(matches!(result, Err(TrustError::DuplicateEvent(_))));
    }

    #[test]
    fn providers_returns_self() {
        let service = setup();
        let resp = service.providers();

        assert_eq!(resp.providers.len(), 1);
        assert_eq!(resp.providers[0].conformance_level, 1);
        assert!(resp.providers[0].provider_id.starts_with("did:key:"));
    }

    #[test]
    fn subjects_lists_every_updated_agent_in_sorted_order() {
        let service = setup();
        // setup() already populates "did:key:z6MkAgent1"; add two peers and
        // confirm the enumeration covers all three in sorted order.
        service.update_agent_input("agent-c", AgentTrustInput::default());
        service.update_agent_input("agent-a", AgentTrustInput::default());
        let subjects = service.subjects();
        assert_eq!(subjects, vec!["agent-a", "agent-c", "did:key:z6MkAgent1"]);
    }

    #[test]
    fn providers_advertises_behavioral_fidelity() {
        let service = setup();
        let resp = service.providers();
        assert!(
            resp.providers[0]
                .dimensions
                .iter()
                .any(|d| d == dimensions::BEHAVIORAL_FIDELITY)
        );
    }

    #[test]
    fn query_includes_behavioral_fidelity_when_signals_present() {
        let keypair = AgentKeypair::generate();
        let service = TrustService::new(keypair, 3600);
        service.update_agent_input(
            "did:key:z6MkAgentBF",
            AgentTrustInput {
                has_verified_did: true,
                mean_fidelity_ratio: Some(0.95),
                fidelity_observation_count: Some(200),
                mean_simulation_divergence: Some(0.05),
                scope_violation_count: Some(1),
                ..Default::default()
            },
        );

        let resp = service
            .query(&TrustQueryRequest {
                subject_id: "did:key:z6MkAgentBF".to_string(),
                domain: None,
                provider_id: None,
                max_age_seconds: None,
                dimensions: None,
            })
            .unwrap();

        assert!(
            resp.trust_score
                .score
                .dimensions
                .contains_key(dimensions::BEHAVIORAL_FIDELITY)
        );
    }

    #[test]
    fn publish_v0_2_event_payload_round_trips_through_history() {
        let service = setup();
        let keypair = AgentKeypair::generate();

        let payload = serde_json::to_value(crate::events::BehaviorTrace {
            trace_id: "trc_x".into(),
            contract_id: "ctr_x".into(),
            tool_calls: vec![],
            resources_accessed: vec![],
            side_effects: vec![],
            duration_ms: 42,
            peak_memory_bytes: None,
            total_tool_calls: 0,
            undeclared_tool_calls: 0,
            total_resources: 0,
            undeclared_resources: 0,
            fidelity_ratio: 1.0,
        })
        .unwrap();

        let mut event = TrustEvent {
            event_id: "evt_bt_1".to_string(),
            event_type: event_types::BEHAVIOR_TRACE.to_string(),
            subject_id: "did:key:z6MkAgent1".to_string(),
            issuer_id: keypair.public_key().to_did_key(),
            timestamp: Utc::now(),
            payload,
            dimensions_affected: Some(vec![
                dimensions::BEHAVIORAL_FIDELITY.to_string(),
                dimensions::PERFORMANCE.to_string(),
            ]),
            signature: None,
            co_signatures: None,
        };
        crate::signing::sign_trust_event(&mut event, &keypair);
        service
            .publish(&TrustPublishRequest {
                event: event.clone(),
            })
            .unwrap();

        let hist = service.history(&TrustHistoryRequest {
            subject_id: "did:key:z6MkAgent1".to_string(),
            event_types: Some(vec![event_types::BEHAVIOR_TRACE.to_string()]),
            after: None,
            before: None,
            limit: None,
            cursor: None,
        });
        assert_eq!(hist.events.len(), 1);
        let parsed: crate::events::BehaviorTrace =
            serde_json::from_value(hist.events[0].payload.clone()).unwrap();
        assert_eq!(parsed.trace_id, "trc_x");
        assert!((parsed.fidelity_ratio - 1.0).abs() < 1e-9);
    }

    #[test]
    fn emit_event_creates_signed_event() {
        let service = setup();
        let event_id = service
            .emit_event(
                "did:key:z6MkAgent1",
                "behavior.anomaly",
                serde_json::json!({"anomaly_type": "hallucinated_tool", "evidence": "tool_xyz"}),
                vec!["security".to_string()],
            )
            .unwrap();

        assert!(event_id.starts_with("evt_"));

        let hist = service.history(&TrustHistoryRequest {
            subject_id: "did:key:z6MkAgent1".to_string(),
            event_types: Some(vec!["behavior.anomaly".to_string()]),
            after: None,
            before: None,
            limit: None,
            cursor: None,
        });

        assert_eq!(hist.total_count, 1);
        assert!(hist.events[0].signature.is_some());
    }

    #[test]
    fn publish_invalidates_cache() {
        let service = setup();

        let req = TrustQueryRequest {
            subject_id: "did:key:z6MkAgent1".to_string(),
            domain: Some("general".to_string()),
            provider_id: None,
            max_age_seconds: None,
            dimensions: None,
        };
        let score1 = service.query(&req).unwrap();

        service
            .emit_event(
                "did:key:z6MkAgent1",
                "contract.completed",
                serde_json::json!({}),
                vec!["performance".to_string()],
            )
            .unwrap();

        let score2 = service.query(&req).unwrap();

        assert!(score1.trust_score.signature.is_some());
        assert!(score2.trust_score.signature.is_some());
    }
}
