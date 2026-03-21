#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaintClassification {
    Public,
    Internal,
    Pii,
    Credentials,
}

#[derive(Debug, Clone)]
pub struct PayloadTaint {
    pub source_id: String,
    pub classification: TaintClassification,
    pub provenance_chain: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub enum EgressDecision {
    Allow,
    Block { reason: String },
    RequiresAuthorization { reason: String },
}

pub struct EgressTaintGate;

impl EgressTaintGate {
    pub fn new() -> Self {
        Self
    }

    pub fn evaluate(&self, destination: &str, taint_labels: &[PayloadTaint]) -> EgressDecision {
        let is_internal_dest = destination.contains(".company.com")
            || destination.starts_with("https://internal.")
            || destination.starts_with("http://internal.");

        for taint in taint_labels {
            match taint.classification {
                TaintClassification::Credentials => {
                    return EgressDecision::Block {
                        reason: format!(
                            "credential data from {} blocked unconditionally",
                            taint.source_id
                        ),
                    };
                }
                TaintClassification::Internal if !is_internal_dest => {
                    return EgressDecision::Block {
                        reason: format!(
                            "internal data from {} blocked to external endpoint",
                            taint.source_id
                        ),
                    };
                }
                TaintClassification::Pii => {
                    return EgressDecision::RequiresAuthorization {
                        reason: format!("PII data from {} requires authorization", taint.source_id),
                    };
                }
                _ => {}
            }
        }

        EgressDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn internal_taint() -> PayloadTaint {
        PayloadTaint {
            source_id: "intranet".into(),
            classification: TaintClassification::Internal,
            provenance_chain: vec!["intranet".into()],
        }
    }

    fn pii_taint() -> PayloadTaint {
        PayloadTaint {
            source_id: "hr_db".into(),
            classification: TaintClassification::Pii,
            provenance_chain: vec!["hr_db".into()],
        }
    }

    fn credential_taint() -> PayloadTaint {
        PayloadTaint {
            source_id: "vault".into(),
            classification: TaintClassification::Credentials,
            provenance_chain: vec!["vault".into()],
        }
    }

    fn public_taint() -> PayloadTaint {
        PayloadTaint {
            source_id: "web".into(),
            classification: TaintClassification::Public,
            provenance_chain: vec!["web".into()],
        }
    }

    #[spec("SEQ-003")]
    #[test]
    fn blocks_internal_data_to_external_endpoint() {
        let gate = EgressTaintGate::new();
        let decision = gate.evaluate("https://external.com/api", &[internal_taint()]);
        assert!(matches!(decision, EgressDecision::Block { .. }));
    }

    #[spec("SEQ-003")]
    #[test]
    fn blocks_credential_data_unconditionally() {
        let gate = EgressTaintGate::new();
        let decision = gate.evaluate("https://any-destination.com", &[credential_taint()]);
        assert!(matches!(decision, EgressDecision::Block { .. }));
    }

    #[spec("SEQ-003")]
    #[test]
    fn requires_authorization_for_pii_data() {
        let gate = EgressTaintGate::new();
        let decision = gate.evaluate("https://partner.com/api", &[pii_taint()]);
        assert!(matches!(
            decision,
            EgressDecision::RequiresAuthorization { .. }
        ));
    }

    #[spec("SEQ-003")]
    #[test]
    fn allows_public_data_to_external() {
        let gate = EgressTaintGate::new();
        let decision = gate.evaluate("https://external.com/api", &[public_taint()]);
        assert_eq!(decision, EgressDecision::Allow);
    }

    #[spec("SEQ-003")]
    #[test]
    fn allows_internal_data_to_sanctioned_endpoint() {
        let gate = EgressTaintGate::new();
        let decision = gate.evaluate("https://internal.company.com/api", &[internal_taint()]);
        assert_eq!(decision, EgressDecision::Allow);
    }

    #[spec("SEQ-003")]
    #[test]
    fn blocks_encoded_tainted_data() {
        let taint = PayloadTaint {
            source_id: "vault".into(),
            classification: TaintClassification::Credentials,
            provenance_chain: vec!["vault".into(), "base64_encode".into()],
        };
        let gate = EgressTaintGate::new();
        let decision = gate.evaluate("https://external.com/api", &[taint]);
        assert!(matches!(decision, EgressDecision::Block { .. }));
    }

    #[spec("SEQ-003")]
    #[test]
    fn detects_split_tainted_data_across_multiple_requests() {
        let gate = EgressTaintGate::new();
        let chunk1 = PayloadTaint {
            source_id: "vault".into(),
            classification: TaintClassification::Credentials,
            provenance_chain: vec!["vault".into(), "chunk:1/3".into()],
        };
        let chunk2 = PayloadTaint {
            source_id: "vault".into(),
            classification: TaintClassification::Credentials,
            provenance_chain: vec!["vault".into(), "chunk:2/3".into()],
        };
        let decision1 = gate.evaluate("https://external.com/api", &[chunk1]);
        let decision2 = gate.evaluate("https://external.com/api", &[chunk2]);
        assert!(matches!(decision1, EgressDecision::Block { .. }));
        assert!(matches!(decision2, EgressDecision::Block { .. }));
    }
}
