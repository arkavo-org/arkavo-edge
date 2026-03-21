use crate::egress_taint::PayloadTaint;
use crate::url::EgressFilter;

/// Decision from provenance-enhanced egress evaluation
#[derive(Debug, PartialEq)]
pub enum ProvenanceDecision {
    Allow,
    Block { reason: String, provenance: Vec<String> },
}

/// SEQ-014: Enhances EgressFilter with provenance tracking
pub struct ProvenanceEgressFilter {
    _base_filter: EgressFilter,
}

impl ProvenanceEgressFilter {
    pub fn new(base_filter: EgressFilter) -> Self {
        Self {
            _base_filter: base_filter,
        }
    }

    /// Evaluate egress with both IP/domain checks and provenance
    pub fn evaluate_with_provenance(
        &self,
        _url: &str,
        _taint_labels: &[PayloadTaint],
    ) -> ProvenanceDecision {
        ProvenanceDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egress_taint::TaintClassification;
    use arkavo_test_macros::spec;

    fn internal_taint_with_provenance() -> PayloadTaint {
        PayloadTaint {
            source_id: "intranet".into(),
            classification: TaintClassification::Internal,
            provenance_chain: vec![
                "source:intranet".into(),
                "transform:summarize".into(),
            ],
        }
    }

    // =========================================================================
    // SEQ-014: Egress filter enhanced with provenance
    // =========================================================================

    #[spec("SEQ-014")]
    #[test]
    fn existing_ip_checks_still_applied() {
        let base = EgressFilter::new();
        let filter = ProvenanceEgressFilter::new(base);
        let result = filter.evaluate_with_provenance(
            "http://10.0.0.1/api",
            &[],
        );
        assert!(matches!(result, ProvenanceDecision::Block { .. }));
    }

    #[spec("SEQ-014")]
    #[test]
    fn public_data_to_external_passes() {
        let base = EgressFilter::new();
        let filter = ProvenanceEgressFilter::new(base);
        let public = PayloadTaint {
            source_id: "web".into(),
            classification: TaintClassification::Public,
            provenance_chain: vec!["source:web".into()],
        };
        let result = filter.evaluate_with_provenance(
            "https://api.example.com/data",
            &[public],
        );
        assert_eq!(result, ProvenanceDecision::Allow);
    }

    #[spec("SEQ-014")]
    #[test]
    fn internal_tainted_data_blocked_from_external() {
        let base = EgressFilter::new();
        let filter = ProvenanceEgressFilter::new(base);
        let result = filter.evaluate_with_provenance(
            "https://external.com/api",
            &[internal_taint_with_provenance()],
        );
        assert!(matches!(result, ProvenanceDecision::Block { .. }));
    }

    #[spec("SEQ-014")]
    #[test]
    fn block_includes_provenance_chain_for_forensics() {
        let base = EgressFilter::new();
        let filter = ProvenanceEgressFilter::new(base);
        let result = filter.evaluate_with_provenance(
            "https://external.com/api",
            &[internal_taint_with_provenance()],
        );
        if let ProvenanceDecision::Block { provenance, .. } = result {
            assert!(!provenance.is_empty());
            assert!(provenance.iter().any(|p| p.contains("intranet")));
        } else {
            panic!("expected Block decision");
        }
    }

    #[spec("SEQ-014")]
    #[test]
    fn taint_overrides_destination_allowlist() {
        let base = EgressFilter::new();
        let filter = ProvenanceEgressFilter::new(base);
        let credential_taint = PayloadTaint {
            source_id: "vault".into(),
            classification: TaintClassification::Credentials,
            provenance_chain: vec!["source:vault".into()],
        };
        let result = filter.evaluate_with_provenance(
            "https://allowed-partner.com/api",
            &[credential_taint],
        );
        assert!(matches!(result, ProvenanceDecision::Block { .. }));
    }

    #[spec("SEQ-014")]
    #[test]
    fn indeterminate_taint_blocks_conservatively() {
        let base = EgressFilter::new();
        let filter = ProvenanceEgressFilter::new(base);
        let result = filter.evaluate_with_provenance(
            "https://external.com/api",
            &[],
        );
        assert!(matches!(result, ProvenanceDecision::Block { .. }));
    }
}
