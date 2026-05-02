//! Map internal agent scoring data to MCP-T trust dimensions

use chrono::{DateTime, Utc};
use std::collections::HashMap;

use crate::types::{
    DimensionScore, SCHEMA_VERSION, TrustScore, TrustScoreValue, ValidityWindow, dimensions,
};

/// Input data for computing trust dimensions from internal agent scoring.
///
/// Fields are optional — dimensions are only computed for available data.
#[derive(Debug, Clone, Default)]
pub struct AgentTrustInput {
    /// BetaPrior expected_value (0.0-1.0)
    pub success_rate: Option<f64>,
    /// BetaPrior std_dev
    pub success_std_dev: Option<f64>,
    /// Total observations (alpha + beta - priors)
    pub observation_count: Option<u32>,
    /// Whether the agent has a verified DID:key
    pub has_verified_did: bool,
    /// Agent creation timestamp
    pub created_at: Option<DateTime<Utc>>,
    /// Anti-pattern weight for security category (decayed sum)
    pub security_anti_pattern_weight: Option<f64>,
    /// Anti-pattern weight for all categories (decayed sum)
    pub total_anti_pattern_weight: Option<f64>,
    /// Fraction of decisions with traces (0.0-1.0)
    pub decision_trace_coverage: Option<f64>,
    /// Number of connected peers
    pub peer_count: Option<u32>,
    /// Mean of `behavior.trace.fidelity_ratio` across observed traces (0.0-1.0).
    /// 1.0 = every action observed was within declared scope.
    pub mean_fidelity_ratio: Option<f64>,
    /// Number of behavioral traces contributing to `mean_fidelity_ratio`.
    pub fidelity_observation_count: Option<u32>,
    /// Mean simulation/actual divergence (0.0 = perfect prediction, 1.0 = worst).
    /// Sourced from `simulation.delta` events.
    pub mean_simulation_divergence: Option<f64>,
    /// Count of declaration-delta events with severity high or critical.
    pub scope_violation_count: Option<u32>,
}

/// Compute a TrustScore from internal agent data.
pub fn compute_trust_score(
    input: &AgentTrustInput,
    subject_id: &str,
    provider_id: &str,
    domain: Option<&str>,
    max_age_seconds: u32,
) -> TrustScore {
    let mut dimensions = HashMap::new();

    let obs = input.observation_count.unwrap_or(0);

    // Performance: from BetaPrior expected_value
    if let Some(rate) = input.success_rate {
        dimensions.insert(
            dimensions::PERFORMANCE.to_string(),
            DimensionScore {
                value: scale_unit(rate),
                confidence: observation_confidence(obs),
                evidence_count: obs,
            },
        );
    }

    // Consistency: inverse of behavioral variance
    if let Some(std_dev) = input.success_std_dev {
        let consistency = 1.0 - (std_dev.min(0.5) * 2.0);
        dimensions.insert(
            dimensions::CONSISTENCY.to_string(),
            DimensionScore {
                value: scale_unit(consistency),
                confidence: observation_confidence(obs),
                evidence_count: obs,
            },
        );
    }

    // Verification: binary based on DID:key
    dimensions.insert(
        dimensions::VERIFICATION.to_string(),
        DimensionScore {
            value: if input.has_verified_did { 1000 } else { 0 },
            confidence: 1.0,
            evidence_count: u32::from(input.has_verified_did),
        },
    );

    // Tenure: logarithmic age scaling
    if let Some(created) = input.created_at {
        let hours = (Utc::now() - created).num_hours().max(0) as f64;
        let tenure = (hours + 1.0).log2() / (8760.0_f64 + 1.0).log2();
        dimensions.insert(
            dimensions::TENURE.to_string(),
            DimensionScore {
                value: scale_unit(tenure.min(1.0)),
                confidence: 1.0,
                evidence_count: 1,
            },
        );
    }

    // Security: inverse of anti-pattern weight
    if let Some(weight) = input.security_anti_pattern_weight {
        let security = 1.0 - (weight * 0.2).min(1.0);
        dimensions.insert(
            dimensions::SECURITY.to_string(),
            DimensionScore {
                value: scale_unit(security),
                confidence: observation_confidence(obs),
                evidence_count: obs,
            },
        );
    }

    // Transparency: decision trace coverage
    if let Some(coverage) = input.decision_trace_coverage {
        dimensions.insert(
            dimensions::TRANSPARENCY.to_string(),
            DimensionScore {
                value: scale_unit(coverage),
                confidence: observation_confidence(obs),
                evidence_count: obs,
            },
        );
    }

    // Community: peer count
    if let Some(peers) = input.peer_count {
        let community = (peers.min(50) as f64) / 50.0;
        dimensions.insert(
            dimensions::COMMUNITY.to_string(),
            DimensionScore {
                value: scale_unit(community),
                confidence: observation_confidence(peers),
                evidence_count: peers,
            },
        );
    }

    // Compliance: inverse of total anti-pattern weight
    if let Some(weight) = input.total_anti_pattern_weight {
        let compliance = 1.0 - (weight * 0.1).min(1.0);
        dimensions.insert(
            dimensions::COMPLIANCE.to_string(),
            DimensionScore {
                value: scale_unit(compliance),
                confidence: observation_confidence(obs),
                evidence_count: obs,
            },
        );
    }

    // Behavioral fidelity (v0.2.0): combines declared/observed alignment,
    // simulation accuracy, and a penalty for high-severity scope violations.
    // Each signal is independently optional; the score uses whichever are set.
    if let Some(score) = compute_behavioral_fidelity(input) {
        dimensions.insert(dimensions::BEHAVIORAL_FIDELITY.to_string(), score);
    }

    let composite = compute_composite(&dimensions);
    let now = Utc::now();

    TrustScore {
        schema_version: SCHEMA_VERSION.to_string(),
        subject_id: subject_id.to_string(),
        provider_id: provider_id.to_string(),
        score: TrustScoreValue {
            composite,
            dimensions,
        },
        domain: domain.map(|d| d.to_string()),
        domain_match: domain.map(|_| true),
        validity: ValidityWindow {
            issued_at: now,
            expires_at: now + chrono::Duration::seconds(max_age_seconds as i64),
            max_age_seconds,
        },
        metadata: None,
        authorized: Some(true),
        signature: None,
    }
}

/// Scale a 0.0-1.0 value to 0-1000 integer
fn scale_unit(value: f64) -> u32 {
    (value.clamp(0.0, 1.0) * 1000.0) as u32
}

/// Compute confidence from observation count.
/// Asymptotically approaches 1.0 with more observations.
fn observation_confidence(observations: u32) -> f64 {
    1.0 - 1.0 / (observations as f64 + 1.0)
}

/// Combine declared/observed alignment, simulation accuracy, and scope-violation
/// penalty into a single behavioral_fidelity dimension. Returns `None` when no
/// fidelity signals are available — verification-only profiles do not get a
/// score for a dimension they have no evidence for.
fn compute_behavioral_fidelity(input: &AgentTrustInput) -> Option<DimensionScore> {
    let mut weighted_sum = 0.0_f64;
    let mut weight_total = 0.0_f64;
    let mut evidence_total: u32 = 0;

    if let Some(ratio) = input.mean_fidelity_ratio {
        let n = input.fidelity_observation_count.unwrap_or(0);
        let weight = observation_confidence(n).max(0.05);
        weighted_sum += ratio.clamp(0.0, 1.0) * weight;
        weight_total += weight;
        evidence_total = evidence_total.saturating_add(n);
    }

    if let Some(div) = input.mean_simulation_divergence {
        // Convert divergence (0.0 = perfect) to alignment score (1.0 = perfect).
        let alignment = 1.0 - div.clamp(0.0, 1.0);
        // Simulation accuracy is meaningful even with few samples; weight it
        // moderately so it does not dominate fidelity_ratio.
        let weight = 0.5;
        weighted_sum += alignment * weight;
        weight_total += weight;
    }

    if let Some(violations) = input.scope_violation_count {
        // Each high/critical violation subtracts proportionally from fidelity,
        // capped at 1.0 (i.e. ten or more violations zero this signal out).
        let alignment = 1.0 - (violations as f64 * 0.1).min(1.0);
        let weight = 0.75;
        weighted_sum += alignment * weight;
        weight_total += weight;
        evidence_total = evidence_total.saturating_add(violations);
    }

    if weight_total == 0.0 {
        return None;
    }

    let value = scale_unit(weighted_sum / weight_total);
    let confidence = observation_confidence(evidence_total);
    Some(DimensionScore {
        value,
        confidence,
        evidence_count: evidence_total,
    })
}

/// Compute composite score as weighted mean of dimensions.
fn compute_composite(dimensions: &HashMap<String, DimensionScore>) -> u32 {
    if dimensions.is_empty() {
        return 0;
    }

    let total_weight: f64 = dimensions.values().map(|d| d.confidence).sum();
    if total_weight == 0.0 {
        return 0;
    }

    let weighted_sum: f64 = dimensions
        .values()
        .map(|d| d.value as f64 * d.confidence)
        .sum();

    (weighted_sum / total_weight) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_unit_boundaries() {
        assert_eq!(scale_unit(0.0), 0);
        assert_eq!(scale_unit(1.0), 1000);
        assert_eq!(scale_unit(0.5), 500);
        assert_eq!(scale_unit(-0.1), 0);
        assert_eq!(scale_unit(1.5), 1000);
    }

    #[test]
    fn observation_confidence_grows() {
        let c0 = observation_confidence(0);
        let c10 = observation_confidence(10);
        let c100 = observation_confidence(100);
        let c1000 = observation_confidence(1000);

        assert!(c0 < c10);
        assert!(c10 < c100);
        assert!(c100 < c1000);
        assert!(c1000 < 1.0);
        assert_eq!(c0, 0.0);
    }

    #[test]
    fn empty_input_produces_verification_only() {
        let input = AgentTrustInput::default();
        let score = compute_trust_score(&input, "did:key:z6MkTest", "did:key:z6MkProv", None, 3600);

        // Only verification dimension (always present, value=0 since has_verified_did is false)
        assert!(
            score
                .score
                .dimensions
                .contains_key(dimensions::VERIFICATION)
        );
        assert_eq!(score.score.dimensions[dimensions::VERIFICATION].value, 0);
    }

    #[test]
    fn full_input_produces_all_dimensions() {
        let input = AgentTrustInput {
            success_rate: Some(0.85),
            success_std_dev: Some(0.1),
            observation_count: Some(200),
            has_verified_did: true,
            created_at: Some(Utc::now() - chrono::Duration::days(90)),
            security_anti_pattern_weight: Some(0.5),
            total_anti_pattern_weight: Some(1.0),
            decision_trace_coverage: Some(0.95),
            peer_count: Some(10),
            mean_fidelity_ratio: Some(0.9),
            fidelity_observation_count: Some(120),
            mean_simulation_divergence: Some(0.1),
            scope_violation_count: Some(0),
        };

        let score = compute_trust_score(
            &input,
            "did:key:z6MkTest",
            "did:key:z6MkProv",
            Some("code-execution"),
            3600,
        );

        assert!(score.score.dimensions.len() >= 6);
        assert_eq!(score.score.dimensions[dimensions::VERIFICATION].value, 1000);
        assert_eq!(score.score.dimensions[dimensions::PERFORMANCE].value, 850);
        assert!(
            score
                .score
                .dimensions
                .contains_key(dimensions::BEHAVIORAL_FIDELITY)
        );
        assert_eq!(score.domain, Some("code-execution".to_string()));
        assert!(score.score.composite > 0);
    }

    #[test]
    fn composite_is_confidence_weighted_mean() {
        let mut dims = HashMap::new();
        dims.insert(
            "a".to_string(),
            DimensionScore {
                value: 1000,
                confidence: 1.0,
                evidence_count: 100,
            },
        );
        dims.insert(
            "b".to_string(),
            DimensionScore {
                value: 0,
                confidence: 1.0,
                evidence_count: 100,
            },
        );
        assert_eq!(compute_composite(&dims), 500);
    }

    #[test]
    fn composite_weights_by_confidence() {
        let mut dims = HashMap::new();
        dims.insert(
            "high_conf".to_string(),
            DimensionScore {
                value: 1000,
                confidence: 0.9,
                evidence_count: 100,
            },
        );
        dims.insert(
            "low_conf".to_string(),
            DimensionScore {
                value: 0,
                confidence: 0.1,
                evidence_count: 1,
            },
        );
        let composite = compute_composite(&dims);
        // Should be heavily weighted toward 1000
        assert!(composite > 800);
    }

    #[test]
    fn validity_window_set_correctly() {
        let input = AgentTrustInput::default();
        let score = compute_trust_score(&input, "did:key:z6MkTest", "did:key:z6MkProv", None, 7200);

        assert_eq!(score.validity.max_age_seconds, 7200);
        assert!(score.validity.expires_at > score.validity.issued_at);
    }

    #[test]
    fn tenure_logarithmic_scaling() {
        // New agent: low tenure
        let new_input = AgentTrustInput {
            created_at: Some(Utc::now() - chrono::Duration::hours(1)),
            ..Default::default()
        };
        let new_score = compute_trust_score(
            &new_input,
            "did:key:z6MkTest",
            "did:key:z6MkProv",
            None,
            3600,
        );

        // Old agent: high tenure
        let old_input = AgentTrustInput {
            created_at: Some(Utc::now() - chrono::Duration::days(365)),
            ..Default::default()
        };
        let old_score = compute_trust_score(
            &old_input,
            "did:key:z6MkTest",
            "did:key:z6MkProv",
            None,
            3600,
        );

        let new_tenure = new_score.score.dimensions[dimensions::TENURE].value;
        let old_tenure = old_score.score.dimensions[dimensions::TENURE].value;

        assert!(old_tenure > new_tenure);
        assert!(old_tenure >= 900); // ~1 year should be close to max
    }

    #[test]
    fn behavioral_fidelity_absent_when_no_signals() {
        let input = AgentTrustInput::default();
        let score = compute_trust_score(&input, "did:key:z6MkTest", "did:key:z6MkProv", None, 3600);
        assert!(
            !score
                .score
                .dimensions
                .contains_key(dimensions::BEHAVIORAL_FIDELITY)
        );
    }

    #[test]
    fn behavioral_fidelity_perfect_signals_score_high() {
        let input = AgentTrustInput {
            mean_fidelity_ratio: Some(1.0),
            fidelity_observation_count: Some(500),
            mean_simulation_divergence: Some(0.0),
            scope_violation_count: Some(0),
            ..Default::default()
        };
        let score = compute_trust_score(&input, "did:key:z6MkTest", "did:key:z6MkProv", None, 3600);
        let bf = &score.score.dimensions[dimensions::BEHAVIORAL_FIDELITY];
        assert_eq!(bf.value, 1000);
        assert!(bf.confidence > 0.99);
    }

    #[test]
    fn behavioral_fidelity_violations_drag_score_down() {
        let clean = AgentTrustInput {
            mean_fidelity_ratio: Some(0.9),
            fidelity_observation_count: Some(100),
            scope_violation_count: Some(0),
            ..Default::default()
        };
        let dirty = AgentTrustInput {
            mean_fidelity_ratio: Some(0.9),
            fidelity_observation_count: Some(100),
            scope_violation_count: Some(10),
            ..Default::default()
        };

        let clean_score =
            compute_trust_score(&clean, "did:key:z6MkTest", "did:key:z6MkProv", None, 3600);
        let dirty_score =
            compute_trust_score(&dirty, "did:key:z6MkTest", "did:key:z6MkProv", None, 3600);

        let clean_bf = clean_score.score.dimensions[dimensions::BEHAVIORAL_FIDELITY].value;
        let dirty_bf = dirty_score.score.dimensions[dimensions::BEHAVIORAL_FIDELITY].value;
        assert!(clean_bf > dirty_bf, "clean={} dirty={}", clean_bf, dirty_bf);
    }

    #[test]
    fn behavioral_fidelity_simulation_only_signal() {
        let input = AgentTrustInput {
            mean_simulation_divergence: Some(0.25),
            ..Default::default()
        };
        let score = compute_trust_score(&input, "did:key:z6MkTest", "did:key:z6MkProv", None, 3600);
        let bf = &score.score.dimensions[dimensions::BEHAVIORAL_FIDELITY];
        // Single signal, alignment = 0.75 → ~750
        assert!((700..=800).contains(&bf.value), "got {}", bf.value);
    }

    #[test]
    fn security_inversely_related_to_antipatterns() {
        let clean = AgentTrustInput {
            security_anti_pattern_weight: Some(0.0),
            ..Default::default()
        };
        let dirty = AgentTrustInput {
            security_anti_pattern_weight: Some(5.0),
            ..Default::default()
        };

        let clean_score =
            compute_trust_score(&clean, "did:key:z6MkTest", "did:key:z6MkProv", None, 3600);
        let dirty_score =
            compute_trust_score(&dirty, "did:key:z6MkTest", "did:key:z6MkProv", None, 3600);

        assert_eq!(
            clean_score.score.dimensions[dimensions::SECURITY].value,
            1000
        );
        assert_eq!(dirty_score.score.dimensions[dimensions::SECURITY].value, 0);
    }
}
