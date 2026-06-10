//! Agent Runtime Policy (ARP) document parser and data model.
//!
//! Provides typed Rust structs for deserializing ARP JSON documents
//! per the ARP specification draft-00. This crate is a data model
//! and parser only — no policy engine or business logic.

pub mod adaptation;
pub mod capability;
pub mod constraints;
pub mod feedback;
pub mod layers;
pub mod model;
pub mod observability;
pub mod proposal;
pub mod validate;

pub use capability::{CapabilityGates, CapabilityProposal, CapabilityRef, CapabilityState};
pub use model::ArpDocument;
pub use proposal::{
    BlastRadius, ProposalOrigin, ProposalPolicy, ProposalState, TighteningEffect,
    TighteningProposal, TraceRef,
};

/// Parse an ARP document from a JSON string.
pub fn parse(json: &str) -> Result<ArpDocument, ParseError> {
    let doc: ArpDocument = serde_json::from_str(json).map_err(ParseError::Json)?;
    validate::validate(&doc)?;
    Ok(doc)
}

/// Errors that can occur when parsing or validating an ARP document.
#[derive(Debug)]
pub enum ParseError {
    Json(serde_json::Error),
    Validation(validate::ValidationError),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "JSON parse error: {e}"),
            Self::Validation(e) => write!(f, "validation error: {e}"),
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(e) => Some(e),
            Self::Validation(e) => Some(e),
        }
    }
}

impl From<validate::ValidationError> for ParseError {
    fn from(e: validate::ValidationError) -> Self {
        Self::Validation(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_document() {
        let json = r#"{
            "arp_spec": "0.1.0",
            "adl_ref": {
                "uri": "https://example.com/agent/adl.json",
                "document_hash": "sha256:abcdef0123456789"
            },
            "adaptation": {
                "method": "thompson_sampling"
            },
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
                        "decay_strategy": "exponential",
                        "decay_half_life_sec": 86400
                    }
                }
            },
            "budget": {
                "task_ceiling_usd": 2.50,
                "on_exhaustion": "halt_and_report",
                "velocity": {
                    "max_spend_per_minute_usd": 0.50
                }
            }
        }"#;

        let doc = parse(json).expect("should parse minimal ARP document");
        assert_eq!(doc.arp_spec, "0.1.0");
        assert_eq!(
            doc.adl_ref.uri.as_deref(),
            Some("https://example.com/agent/adl.json")
        );
        assert_eq!(doc.budget.task_ceiling_usd, 2.50);
    }

    #[test]
    fn reject_missing_required_fields() {
        let json = r#"{"arp_spec": "0.1.0"}"#;
        assert!(parse(json).is_err());
    }

    #[test]
    fn round_trip_serialization() {
        let json = r#"{
            "arp_spec": "0.1.0",
            "adl_ref": {"document_hash": "sha256:abcdef0123456789"},
            "adaptation": {"method": "static"},
            "feedback_loops": {
                "immediate": {
                    "quality_gate": {
                        "threshold_default": 0.5,
                        "metric": "regex_match",
                        "on_failure": "log_only"
                    }
                },
                "short_term": {
                    "policy_cache": {
                        "default_ttl_sec": 1800,
                        "decay_strategy": "linear"
                    }
                }
            },
            "budget": {
                "task_ceiling_usd": 1.0,
                "on_exhaustion": "halt_and_report",
                "velocity": {"max_spend_per_minute_usd": 0.25}
            }
        }"#;

        let doc = parse(json).expect("should parse");
        let serialized = serde_json::to_string(&doc).expect("should serialize");
        let reparsed: ArpDocument = serde_json::from_str(&serialized).expect("should re-parse");
        assert_eq!(doc.arp_spec, reparsed.arp_spec);
        assert_eq!(
            doc.budget.task_ceiling_usd,
            reparsed.budget.task_ceiling_usd
        );
    }

    #[test]
    fn parse_with_all_optional_sections() {
        let json = r#"{
            "arp_spec": "0.1.0",
            "adl_ref": {"uri": "https://example.com/adl.json"},
            "adaptation": {"method": "epsilon_greedy"},
            "feedback_loops": {
                "immediate": {
                    "quality_gate": {
                        "threshold_default": 0.7,
                        "metric": "llm_judge",
                        "on_failure": "update_prior_and_retry"
                    }
                },
                "short_term": {
                    "policy_cache": {
                        "default_ttl_sec": 7200,
                        "decay_strategy": "step"
                    }
                }
            },
            "budget": {
                "task_ceiling_usd": 5.0,
                "on_exhaustion": "degrade_to_cheapest_model",
                "velocity": {"max_spend_per_minute_usd": 1.0}
            },
            "cognitive": {
                "context_inspection": {
                    "pattern_registry": [
                        {
                            "name": "ssn",
                            "regex": "\\d{3}-\\d{2}-\\d{4}",
                            "sensitivity": "restricted",
                            "action": "redact"
                        }
                    ]
                }
            },
            "session": {
                "timeout_bounds": {
                    "absolute_timeout_min_sec": 60,
                    "absolute_timeout_max_sec": 86400
                },
                "defaults": {
                    "absolute_timeout_sec": 3600,
                    "idle_timeout_sec": 900
                }
            },
            "escalation": {
                "ratchet_direction": "tighten_only",
                "relaxation_requires": "human_approval"
            }
        }"#;

        let doc = parse(json).expect("should parse full document");
        let cognitive = doc.cognitive.expect("cognitive should be present");
        let inspection = cognitive
            .context_inspection
            .expect("inspection should be present");
        let patterns = inspection
            .pattern_registry
            .expect("patterns should be present");
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].name, "ssn");
    }
}
