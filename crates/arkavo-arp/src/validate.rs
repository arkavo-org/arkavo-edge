//! Semantic validation for ARP documents beyond JSON schema conformance.

use crate::feedback::DecayStrategy;
use crate::model::{ArpDocument, SignedContent};

/// Validate semantic constraints that cannot be expressed in JSON Schema alone.
pub fn validate(doc: &ArpDocument) -> Result<(), ValidationError> {
    validate_adl_ref(doc)?;
    validate_integrity(doc)?;
    validate_decay_half_life(doc)?;
    validate_beta_priors(doc)?;
    validate_escalation_invariants(doc)?;
    validate_quarantine_invariants(doc)?;
    validate_quality_gate_threshold(doc)?;
    Ok(())
}

fn validate_adl_ref(doc: &ArpDocument) -> Result<(), ValidationError> {
    if doc.adl_ref.uri.is_none() && doc.adl_ref.document_hash.is_none() {
        return Err(ValidationError {
            field: "adl_ref".into(),
            message: "at least one of 'uri' or 'document_hash' is required".into(),
        });
    }
    Ok(())
}

fn validate_integrity(doc: &ArpDocument) -> Result<(), ValidationError> {
    if let Some(integrity) = &doc.integrity
        && integrity.signed_content == SignedContent::Digest
        && (integrity.digest_algorithm.is_none() || integrity.digest_value.is_none())
    {
        return Err(ValidationError {
            field: "integrity".into(),
            message: "digest_algorithm and digest_value required when signed_content is 'digest'"
                .into(),
        });
    }
    Ok(())
}

fn validate_decay_half_life(doc: &ArpDocument) -> Result<(), ValidationError> {
    let cache = &doc.feedback_loops.short_term.policy_cache;
    if cache.decay_strategy == DecayStrategy::Exponential && cache.decay_half_life_sec.is_none() {
        return Err(ValidationError {
            field: "feedback_loops.short_term.policy_cache.decay_half_life_sec".into(),
            message: "decay_half_life_sec is required when decay_strategy is 'exponential'".into(),
        });
    }
    // Note: decay_half_life_sec is silently ignored when decay_strategy is Linear.
    // This is intentional — the field is only meaningful for Exponential decay.
    Ok(())
}

fn validate_beta_priors(doc: &ArpDocument) -> Result<(), ValidationError> {
    if let Some(cold) = &doc.adaptation.cold_start
        && let Some(prior) = &cold.initial_prior
        && (prior.alpha <= 0.0 || prior.beta <= 0.0)
    {
        return Err(ValidationError {
            field: "adaptation.cold_start.initial_prior".into(),
            message: format!(
                "alpha and beta must be positive, got alpha={}, beta={}",
                prior.alpha, prior.beta
            ),
        });
    }
    if let Some(pm) = &doc.adaptation.prior_management
        && let Some(reset) = &pm.reset_state
        && (reset.alpha <= 0.0 || reset.beta <= 0.0)
    {
        return Err(ValidationError {
            field: "adaptation.prior_management.reset_state".into(),
            message: format!(
                "alpha and beta must be positive, got alpha={}, beta={}",
                reset.alpha, reset.beta
            ),
        });
    }
    Ok(())
}

fn validate_escalation_invariants(doc: &ArpDocument) -> Result<(), ValidationError> {
    if let Some(esc) = &doc.escalation {
        if let Some(dir) = &esc.ratchet_direction
            && dir != "tighten_only"
        {
            return Err(ValidationError {
                field: "escalation.ratchet_direction".into(),
                message: "ratchet_direction MUST be 'tighten_only'".into(),
            });
        }
        if let Some(req) = &esc.relaxation_requires
            && req != "human_approval"
        {
            return Err(ValidationError {
                field: "escalation.relaxation_requires".into(),
                message: "relaxation_requires MUST be 'human_approval'".into(),
            });
        }
    }
    Ok(())
}

fn validate_quarantine_invariants(doc: &ArpDocument) -> Result<(), ValidationError> {
    if let Some(q) = &doc.quarantine
        && let Some(sticky) = &q.sticky_requires
        && sticky != "human_review"
    {
        return Err(ValidationError {
            field: "quarantine.sticky_requires".into(),
            message: "sticky_requires MUST be 'human_review'".into(),
        });
    }
    Ok(())
}

fn validate_quality_gate_threshold(doc: &ArpDocument) -> Result<(), ValidationError> {
    let threshold = doc.feedback_loops.immediate.quality_gate.threshold_default;
    if !(0.0..=1.0).contains(&threshold) {
        return Err(ValidationError {
            field: "feedback_loops.immediate.quality_gate.threshold_default".into(),
            message: format!("threshold_default must be 0.0-1.0, got {threshold}"),
        });
    }
    Ok(())
}

/// A semantic validation error.
#[derive(Debug)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl std::error::Error for ValidationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptation::{Adaptation, AdaptationMethod};
    use crate::constraints::{Budget, BudgetExhaustionAction, Velocity};
    use crate::feedback::{
        FeedbackLoops, ImmediateFeedback, PolicyCacheConfig, QualityFailureAction, QualityGate,
        QualityMetric, ShortTermFeedback,
    };
    use crate::model::{AdlRef, BetaPrior};

    fn minimal_doc() -> ArpDocument {
        ArpDocument {
            arp_spec: "0.1.0".into(),
            adl_ref: AdlRef {
                uri: Some("https://example.com/adl.json".into()),
                document_hash: None,
            },
            integrity: None,
            adaptation: Adaptation {
                method: AdaptationMethod::Static,
                parameters: None,
                cold_start: None,
                prior_management: None,
                signal_separation: None,
            },
            feedback_loops: FeedbackLoops {
                immediate: ImmediateFeedback {
                    quality_gate: QualityGate {
                        threshold_default: 0.7,
                        metric: QualityMetric::CosineSimilarity,
                        on_failure: QualityFailureAction::LogOnly,
                        threshold_overrides: None,
                    },
                },
                short_term: ShortTermFeedback {
                    policy_cache: PolicyCacheConfig {
                        default_ttl_sec: 3600,
                        decay_strategy: DecayStrategy::Linear,
                        decay_half_life_sec: None,
                        human_source_exempt_from_decay: None,
                        incident_source_quarantine_sec: None,
                    },
                },
                gossip: None,
                consolidation: None,
                resilience: None,
            },
            precedence: None,
            cognitive: None,
            execution: None,
            data_sovereignty: None,
            network: None,
            budget: Budget {
                task_ceiling_usd: 1.0,
                on_exhaustion: BudgetExhaustionAction::HaltAndReport,
                degradation_chain: None,
                alert_threshold_pct: None,
                velocity: Velocity {
                    max_spend_per_minute_usd: 0.25,
                    max_tool_calls_per_minute: None,
                    max_tokens_per_minute: None,
                },
                per_layer: None,
                rate_limiting: None,
                accounting: None,
            },
            escalation: None,
            quarantine: None,
            hitl: None,
            session: None,
            state_storage: None,
            observability: None,
            metadata: None,
        }
    }

    #[test]
    fn valid_minimal_document() {
        let doc = minimal_doc();
        assert!(validate(&doc).is_ok());
    }

    #[test]
    fn reject_adl_ref_without_uri_or_hash() {
        let mut doc = minimal_doc();
        doc.adl_ref = AdlRef {
            uri: None,
            document_hash: None,
        };
        let err = validate(&doc).unwrap_err();
        assert_eq!(err.field, "adl_ref");
    }

    #[test]
    fn reject_exponential_decay_without_half_life() {
        let mut doc = minimal_doc();
        doc.feedback_loops.short_term.policy_cache.decay_strategy = DecayStrategy::Exponential;
        doc.feedback_loops
            .short_term
            .policy_cache
            .decay_half_life_sec = None;
        let err = validate(&doc).unwrap_err();
        assert!(err.field.contains("decay_half_life_sec"));
    }

    #[test]
    fn accept_exponential_decay_with_half_life() {
        let mut doc = minimal_doc();
        doc.feedback_loops.short_term.policy_cache.decay_strategy = DecayStrategy::Exponential;
        doc.feedback_loops
            .short_term
            .policy_cache
            .decay_half_life_sec = Some(86400);
        assert!(validate(&doc).is_ok());
    }

    #[test]
    fn reject_invalid_ratchet_direction() {
        use crate::constraints::Escalation;
        let mut doc = minimal_doc();
        doc.escalation = Some(Escalation {
            cross_layer_rules: None,
            ratchet_direction: Some("loosen_allowed".into()),
            relaxation_requires: None,
        });
        let err = validate(&doc).unwrap_err();
        assert!(err.field.contains("ratchet_direction"));
    }

    #[test]
    fn reject_invalid_sticky_requires() {
        use crate::constraints::Quarantine;
        let mut doc = minimal_doc();
        doc.quarantine = Some(Quarantine {
            default_ttl_sec: Some(3600),
            confidence_threshold_for_sticky: None,
            ttl_formula: None,
            sticky_requires: Some("automatic".into()),
            scopes: None,
        });
        let err = validate(&doc).unwrap_err();
        assert!(err.field.contains("sticky_requires"));
    }

    #[test]
    fn reject_zero_alpha_in_initial_prior() {
        use crate::adaptation::{Adaptation, AdaptationMethod, ColdStart};
        let mut doc = minimal_doc();
        doc.adaptation = Adaptation {
            method: AdaptationMethod::ThompsonSampling,
            parameters: None,
            cold_start: Some(ColdStart {
                strategy: None,
                initial_prior: Some(BetaPrior {
                    alpha: 0.0,
                    beta: 1.0,
                }),
                warmup_period: None,
                warmup_behavior: None,
            }),
            prior_management: None,
            signal_separation: None,
        };
        let err = validate(&doc).unwrap_err();
        assert!(err.field.contains("initial_prior"));
    }

    #[test]
    fn reject_negative_beta_in_reset_state() {
        use crate::adaptation::{Adaptation, AdaptationMethod, PriorManagement};
        let mut doc = minimal_doc();
        doc.adaptation = Adaptation {
            method: AdaptationMethod::ThompsonSampling,
            parameters: None,
            cold_start: None,
            prior_management: Some(PriorManagement {
                version_binding: None,
                reset_on_version_change: None,
                reset_state: Some(BetaPrior {
                    alpha: 1.0,
                    beta: -0.5,
                }),
            }),
            signal_separation: None,
        };
        let err = validate(&doc).unwrap_err();
        assert!(err.field.contains("reset_state"));
    }
}
