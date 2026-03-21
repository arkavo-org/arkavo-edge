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

    /// SEQ-003: Evaluate egress request against taint policy
    pub fn evaluate(
        &self,
        _destination: &str,
        _taint_labels: &[PayloadTaint],
    ) -> EgressDecision {
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

    // =========================================================================
    // SEQ-003: Block tainted data exfiltration at egress
    // =========================================================================

    #[spec("SEQ-003")]
    #[test]
    fn internal_data_blocked_from_external_endpoint() {
        let gate = EgressTaintGate::new();
        let decision = gate.evaluate("https://external.com/api", &[internal_taint()]);
        assert!(matches!(decision, EgressDecision::Block { .. }));
    }

    #[spec("SEQ-003")]
    #[test]
    fn credential_data_blocked_unconditionally() {
        let gate = EgressTaintGate::new();
        let decision = gate.evaluate("https://any-destination.com", &[credential_taint()]);
        assert!(matches!(decision, EgressDecision::Block { .. }));
    }

    #[spec("SEQ-003")]
    #[test]
    fn pii_data_requires_authorization() {
        let gate = EgressTaintGate::new();
        let decision = gate.evaluate("https://partner.com/api", &[pii_taint()]);
        assert!(matches!(decision, EgressDecision::RequiresAuthorization { .. }));
    }

    #[spec("SEQ-003")]
    #[test]
    fn public_data_allowed_to_external() {
        let gate = EgressTaintGate::new();
        let decision = gate.evaluate("https://external.com/api", &[public_taint()]);
        assert_eq!(decision, EgressDecision::Allow);
    }

    #[spec("SEQ-003")]
    #[test]
    fn sanctioned_internal_endpoint_allows_internal_data() {
        let gate = EgressTaintGate::new();
        let decision = gate.evaluate("https://internal.company.com/api", &[internal_taint()]);
        assert_eq!(decision, EgressDecision::Allow);
    }

    #[spec("SEQ-003")]
    #[test]
    fn encoded_tainted_data_still_blocked() {
        let taint = PayloadTaint {
            source_id: "vault".into(),
            classification: TaintClassification::Credentials,
            provenance_chain: vec!["vault".into(), "base64_encode".into()],
        };
        let gate = EgressTaintGate::new();
        let decision = gate.evaluate("https://external.com/api", &[taint]);
        assert!(matches!(decision, EgressDecision::Block { .. }));
    }
}
