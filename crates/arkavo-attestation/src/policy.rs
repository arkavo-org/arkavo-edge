//! Acceptance policy: whether an attestation may be accepted for a domain.
//!
//! Specs: specs/arkavo-edge/hardware-attestation.spec.yaml (HATT-011).

use crate::SecurityState;
use crate::attestation_key::AssuranceTier;

/// Criticality of the domain an attestation is being accepted for.
///
/// High-criticality domains demand a Trusted-eligible hardware tier and a
/// non-Compromised platform; Standard domains tolerate weaker assurance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainCriticality {
    /// Sensitive domain: only hardware-rooted, non-Compromised platforms.
    High,
    /// Everyday domain: accept unless the platform is Compromised.
    Standard,
}

/// Whether an attestation at `state`/`tier` is acceptable for `domain`.
///
/// A `Compromised` platform is never acceptable, regardless of domain. High
/// criticality additionally requires a Trusted-eligible (hardware-rooted) tier;
/// Standard criticality accepts any non-Compromised platform.
pub fn attestation_acceptable(
    state: SecurityState,
    tier: AssuranceTier,
    domain: DomainCriticality,
) -> bool {
    if state == SecurityState::Compromised {
        return false;
    }
    match domain {
        DomainCriticality::High => tier.trusted_eligible(),
        DomainCriticality::Standard => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("HATT-011")]
    #[test]
    fn test_compromised_rejected_for_high_criticality() {
        assert!(!attestation_acceptable(
            SecurityState::Compromised,
            AssuranceTier::High,
            DomainCriticality::High,
        ));
    }

    #[spec("HATT-011")]
    #[test]
    fn test_trusted_high_tier_accepted_for_high_criticality() {
        assert!(attestation_acceptable(
            SecurityState::Trusted,
            AssuranceTier::High,
            DomainCriticality::High,
        ));
    }

    #[spec("HATT-011")]
    #[test]
    fn test_software_tier_rejected_for_high_criticality() {
        assert!(!attestation_acceptable(
            SecurityState::Trusted,
            AssuranceTier::None,
            DomainCriticality::High,
        ));
    }

    #[spec("HATT-011")]
    #[test]
    fn test_standard_domain_accepts_non_compromised_software() {
        assert!(attestation_acceptable(
            SecurityState::Unknown,
            AssuranceTier::None,
            DomainCriticality::Standard,
        ));
    }

    #[spec("HATT-011")]
    #[test]
    fn test_standard_domain_rejects_compromised() {
        assert!(!attestation_acceptable(
            SecurityState::Compromised,
            AssuranceTier::High,
            DomainCriticality::Standard,
        ));
    }
}
