use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// MCP-T Trust Score (Section 4.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustScore {
    pub schema_version: String,
    pub subject_id: String,
    pub provider_id: String,
    pub score: TrustScoreValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_match: Option<bool>,
    pub validity: ValidityWindow,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorized: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<TrustSignature>,
}

/// Composite + dimensional scores (Section 4.2.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustScoreValue {
    pub composite: u32,
    pub dimensions: HashMap<String, DimensionScore>,
}

/// Per-dimension score with confidence (Section 4.2.3)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionScore {
    pub value: u32,
    pub confidence: f64,
    pub evidence_count: u32,
}

/// Temporal validity bounds (Section 4.2.4)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidityWindow {
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub max_age_seconds: u32,
}

/// Cryptographic signature (Section 4.2.5)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustSignature {
    pub algorithm: String,
    pub public_key: String,
    pub value: String,
}

/// Trust Event (Section 6.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustEvent {
    pub event_id: String,
    pub event_type: String,
    pub subject_id: String,
    pub issuer_id: String,
    pub timestamp: DateTime<Utc>,
    pub payload: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions_affected: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<TrustSignature>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub co_signatures: Option<Vec<CoSignature>>,
}

/// Co-signature from additional attestors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoSignature {
    pub signer_id: String,
    pub algorithm: String,
    pub public_key: String,
    pub value: String,
}

/// Threshold specification for trust/verify (Section 7.2.3)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composite_min: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimension_mins: Option<HashMap<String, u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_evidence_count: Option<u32>,
}

/// Provider descriptor (Section 8.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    pub provider_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    pub transport_bindings: Vec<String>,
    pub conformance_level: u8,
    pub supported_domains: Vec<String>,
    pub dimensions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scoring_methodology_uri: Option<String>,
    pub public_key: String,
}

// --- JSON-RPC request/response types ---

/// trust/query request params (Section 7.1.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustQueryRequest {
    pub subject_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_age_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<Vec<String>>,
}

/// trust/query response (Section 7.1.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustQueryResponse {
    pub trust_score: TrustScore,
}

/// trust/verify request params (Section 7.2.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustVerifyRequest {
    pub subject_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    pub threshold: ThresholdSpec,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
}

/// trust/verify response (Section 7.2.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustVerifyResponse {
    pub verified: bool,
    pub subject_id: String,
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    pub threshold_results: ThresholdResults,
    pub confidence: f64,
    pub checked_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<TrustSignature>,
}

/// Per-threshold check results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdResults {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composite_min: Option<ThresholdCheck>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimension_mins: Option<HashMap<String, ThresholdCheck>>,
}

/// Single threshold check outcome
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdCheck {
    pub required: u32,
    pub met: bool,
}

/// trust/history request params (Section 7.3.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustHistoryRequest {
    pub subject_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// trust/history response (Section 7.3.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustHistoryResponse {
    pub events: Vec<TrustEvent>,
    pub cursor: Option<String>,
    pub has_more: bool,
    pub total_count: u64,
}

/// trust/providers response (Section 7.4.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustProvidersResponse {
    pub providers: Vec<ProviderDescriptor>,
}

/// trust/publish request params (Section 7.5.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustPublishRequest {
    pub event: TrustEvent,
}

/// trust/publish response (Section 7.5.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustPublishResponse {
    pub accepted: bool,
    pub event_id: String,
    pub processing_status: String,
}

/// MCP-T error codes (Sections 7.1.3, 7.5.3, 7.6)
pub mod error_codes {
    pub const SUBJECT_NOT_FOUND: i32 = -32001;
    pub const PROVIDER_UNAVAILABLE: i32 = -32002;
    pub const DOMAIN_NOT_SUPPORTED: i32 = -32003;
    pub const STALE_SCORE: i32 = -32004;
    pub const INVALID_SIGNATURE: i32 = -32010;
    pub const UNKNOWN_EVENT_TYPE: i32 = -32011;
    pub const DUPLICATE_EVENT: i32 = -32012;
    pub const ISSUER_NOT_AUTHORIZED: i32 = -32013;
    pub const RATE_LIMITED: i32 = -32020;
}

/// Standard domain identifiers (Section 4.4)
pub mod domains {
    pub const GENERAL: &str = "general";
    pub const CODE_EXECUTION: &str = "code-execution";
    pub const FINANCIAL: &str = "financial";
    pub const DATA_ACCESS: &str = "data-access";
    pub const COMMUNICATION: &str = "communication";
    pub const SYSTEM_ADMIN: &str = "system-admin";
    pub const CONTENT_GENERATION: &str = "content-generation";
    pub const RESEARCH: &str = "research";
}

/// Standard trust dimension names (Section 5.1)
pub mod dimensions {
    pub const VERIFICATION: &str = "verification";
    pub const TENURE: &str = "tenure";
    pub const PERFORMANCE: &str = "performance";
    pub const COMMITMENT: &str = "commitment";
    pub const COMMUNITY: &str = "community";
    pub const CONSISTENCY: &str = "consistency";
    pub const TRANSPARENCY: &str = "transparency";
    pub const COMPLIANCE: &str = "compliance";
    pub const SECURITY: &str = "security";
}

pub const SCHEMA_VERSION: &str = "0.1.0";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_score_roundtrip() {
        let mut dims = HashMap::new();
        dims.insert(
            "performance".to_string(),
            DimensionScore {
                value: 780,
                confidence: 0.72,
                evidence_count: 156,
            },
        );
        dims.insert(
            "consistency".to_string(),
            DimensionScore {
                value: 800,
                confidence: 0.83,
                evidence_count: 892,
            },
        );

        let score = TrustScore {
            schema_version: SCHEMA_VERSION.to_string(),
            subject_id: "did:key:z6MkTest".to_string(),
            provider_id: "did:key:z6MkProvider".to_string(),
            score: TrustScoreValue {
                composite: 742,
                dimensions: dims,
            },
            domain: Some("code-execution".to_string()),
            domain_match: Some(true),
            validity: ValidityWindow {
                issued_at: Utc::now(),
                expires_at: Utc::now() + chrono::Duration::hours(1),
                max_age_seconds: 3600,
            },
            metadata: None,
            authorized: Some(true),
            signature: None,
        };

        let json = serde_json::to_string(&score).unwrap();
        let parsed: TrustScore = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.score.composite, 742);
        assert_eq!(parsed.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn trust_event_roundtrip() {
        let event = TrustEvent {
            event_id: "evt_01TEST".to_string(),
            event_type: "contract.completed".to_string(),
            subject_id: "did:key:z6MkTest".to_string(),
            issuer_id: "did:key:z6MkIssuer".to_string(),
            timestamp: Utc::now(),
            payload: serde_json::json!({"contract_id": "ctr_1", "outcome": "success"}),
            dimensions_affected: Some(vec!["performance".to_string()]),
            signature: None,
            co_signatures: None,
        };

        let json = serde_json::to_string(&event).unwrap();
        let parsed: TrustEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event_type, "contract.completed");
    }

    #[test]
    fn threshold_spec_requires_at_least_one() {
        let spec = ThresholdSpec {
            composite_min: Some(700),
            dimension_mins: None,
            confidence_min: None,
            min_evidence_count: None,
        };
        assert!(spec.composite_min.is_some() || spec.dimension_mins.is_some());
    }

    #[test]
    fn verify_response_roundtrip() {
        let mut dim_results = HashMap::new();
        dim_results.insert(
            "verification".to_string(),
            ThresholdCheck {
                required: 800,
                met: true,
            },
        );

        let resp = TrustVerifyResponse {
            verified: true,
            subject_id: "did:key:z6MkTest".to_string(),
            provider_id: "did:key:z6MkProvider".to_string(),
            nonce: Some("abc123".to_string()),
            threshold_results: ThresholdResults {
                composite_min: Some(ThresholdCheck {
                    required: 700,
                    met: true,
                }),
                dimension_mins: Some(dim_results),
            },
            confidence: 0.85,
            checked_at: Utc::now(),
            valid_until: None,
            signature: None,
        };

        let json = serde_json::to_string(&resp).unwrap();
        let parsed: TrustVerifyResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.verified);
        assert_eq!(parsed.nonce, Some("abc123".to_string()));
    }

    #[test]
    fn provider_descriptor_roundtrip() {
        let desc = ProviderDescriptor {
            provider_id: "did:key:z6MkProvider".to_string(),
            name: "Test Trust Provider".to_string(),
            description: Some("Test provider".to_string()),
            endpoint: None,
            transport_bindings: vec!["https".to_string()],
            conformance_level: 1,
            supported_domains: vec!["general".to_string(), "code-execution".to_string()],
            dimensions: vec!["performance".to_string(), "consistency".to_string()],
            scoring_methodology_uri: None,
            public_key: "z6MkTest".to_string(),
        };

        let json = serde_json::to_string(&desc).unwrap();
        let parsed: ProviderDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.conformance_level, 1);
    }
}
