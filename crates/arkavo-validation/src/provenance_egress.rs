use crate::egress_taint::{PayloadTaint, TaintClassification};
use crate::url::EgressFilter;

#[derive(Debug, PartialEq)]
pub enum ProvenanceDecision {
    Allow,
    Block {
        reason: String,
        provenance: Vec<String>,
    },
}

pub struct ProvenanceEgressFilter {
    base_filter: EgressFilter,
}

impl ProvenanceEgressFilter {
    pub fn new(base_filter: EgressFilter) -> Self {
        Self { base_filter }
    }

    pub fn evaluate_with_provenance(
        &self,
        url: &str,
        taint_labels: &[PayloadTaint],
    ) -> ProvenanceDecision {
        // Check base IP/domain filters first
        if self.base_filter.is_allowed(url).is_err() {
            return ProvenanceDecision::Block {
                reason: "blocked by base egress filter".into(),
                provenance: Vec::new(),
            };
        }

        // No taint labels = indeterminate tracking gap = conservative block
        if taint_labels.is_empty() {
            return ProvenanceDecision::Block {
                reason: "no taint labels: indeterminate tracking gap".into(),
                provenance: Vec::new(),
            };
        }

        for taint in taint_labels {
            match taint.classification {
                TaintClassification::Credentials => {
                    return ProvenanceDecision::Block {
                        reason: format!("credential data from {}", taint.source_id),
                        provenance: taint.provenance_chain.clone(),
                    };
                }
                TaintClassification::Internal => {
                    return ProvenanceDecision::Block {
                        reason: format!("internal data from {}", taint.source_id),
                        provenance: taint.provenance_chain.clone(),
                    };
                }
                TaintClassification::Pii => {
                    return ProvenanceDecision::Block {
                        reason: format!("PII data from {}", taint.source_id),
                        provenance: taint.provenance_chain.clone(),
                    };
                }
                TaintClassification::Public => {}
            }
        }

        ProvenanceDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn internal_taint_with_provenance() -> PayloadTaint {
        PayloadTaint {
            source_id: "intranet".into(),
            classification: TaintClassification::Internal,
            provenance_chain: vec!["source:intranet".into(), "transform:summarize".into()],
        }
    }

    #[spec("SEQ-014")]
    #[test]
    fn existing_ip_checks_still_applied() {
        let base = EgressFilter::new();
        let filter = ProvenanceEgressFilter::new(base);
        let result = filter.evaluate_with_provenance("http://10.0.0.1/api", &[]);
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
        let result = filter.evaluate_with_provenance("https://api.example.com/data", &[public]);
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
        let result =
            filter.evaluate_with_provenance("https://allowed-partner.com/api", &[credential_taint]);
        assert!(matches!(result, ProvenanceDecision::Block { .. }));
    }

    #[spec("SEQ-014")]
    #[test]
    fn indeterminate_taint_blocks_conservatively() {
        let base = EgressFilter::new();
        let filter = ProvenanceEgressFilter::new(base);
        let result = filter.evaluate_with_provenance("https://external.com/api", &[]);
        assert!(matches!(result, ProvenanceDecision::Block { .. }));
    }
}
