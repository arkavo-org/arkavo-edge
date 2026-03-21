use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Classification {
    Public,
    Internal,
    Pii,
    Credentials,
}

impl Classification {
    fn rank(&self) -> u8 {
        match self {
            Self::Public => 0,
            Self::Internal => 1,
            Self::Pii => 2,
            Self::Credentials => 3,
        }
    }

    fn max(a: &Self, b: &Self) -> Self {
        if a.rank() >= b.rank() {
            a.clone()
        } else {
            b.clone()
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaintLabel {
    pub source_id: String,
    pub classification: Classification,
    pub provenance_chain: Vec<String>,
}

#[derive(Default)]
pub struct DataTaintTracker {
    labels: HashMap<String, TaintLabel>,
}

const PII_PATTERNS: &[&str] = &[
    "ssn",
    "social security",
    "email:",
    "phone:",
    "address:",
    "dob:",
];
const CREDENTIAL_PATTERNS: &[&str] = &[
    "api_key",
    "apikey",
    "secret",
    "password",
    "token",
    "credential",
    "private_key",
    "sk-",
];

impl DataTaintTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tag(&mut self, source_id: &str, data: &[u8]) -> TaintLabel {
        let classification = self.classify(data);
        let label = TaintLabel {
            source_id: source_id.to_string(),
            classification,
            provenance_chain: vec![source_id.to_string()],
        };
        self.labels.insert(source_id.to_string(), label.clone());
        label
    }

    pub fn classify(&self, data: &[u8]) -> Classification {
        let text = String::from_utf8_lossy(data).to_ascii_lowercase();

        let has_credentials = CREDENTIAL_PATTERNS.iter().any(|p| text.contains(p));
        let has_pii = PII_PATTERNS.iter().any(|p| text.contains(p));

        if has_credentials {
            Classification::Credentials
        } else if has_pii {
            Classification::Pii
        } else {
            Classification::Internal
        }
    }

    pub fn propagate(&self, inputs: &[&TaintLabel], transform_type: &str) -> TaintLabel {
        let mut highest = Classification::Public;
        let mut chain = Vec::new();

        for input in inputs {
            highest = Classification::max(&highest, &input.classification);
            chain.push(input.source_id.clone());
        }
        chain.push(transform_type.to_string());

        TaintLabel {
            source_id: inputs
                .first()
                .map_or(String::new(), |l| l.source_id.clone()),
            classification: highest,
            provenance_chain: chain,
        }
    }

    pub fn labels(&self) -> Vec<&TaintLabel> {
        self.labels.values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    // =========================================================================
    // SEQ-001: Tag data with provenance at ingestion
    // =========================================================================

    #[spec("SEQ-001")]
    #[test]
    fn tag_assigns_source_identifier_to_label() {
        let mut tracker = DataTaintTracker::new();
        let label = tracker.tag("email_api", b"user email: alice@example.com");
        assert_eq!(label.source_id, "email_api");
    }

    #[spec("SEQ-001")]
    #[test]
    fn tag_classifies_personal_data_as_pii() {
        let mut tracker = DataTaintTracker::new();
        let label = tracker.tag("hr_database", b"SSN: 123-45-6789");
        assert_eq!(label.classification, Classification::Pii);
    }

    #[spec("SEQ-001")]
    #[test]
    fn tag_classifies_api_keys_as_credentials() {
        let mut tracker = DataTaintTracker::new();
        let label = tracker.tag("config_file", b"api_key=sk-abc123secret");
        assert_eq!(label.classification, Classification::Credentials);
    }

    #[spec("SEQ-001")]
    #[test]
    fn tag_persists_label_in_tracker() {
        let mut tracker = DataTaintTracker::new();
        tracker.tag("intranet", b"internal memo");
        assert_eq!(tracker.labels().len(), 1);
    }

    #[spec("SEQ-001")]
    #[test]
    fn classify_treats_ambiguous_data_conservatively() {
        let tracker = DataTaintTracker::new();
        let classification = tracker.classify(b"some unknown data format");
        assert!(
            classification == Classification::Internal || classification == Classification::Pii,
        );
    }

    #[spec("SEQ-001")]
    #[test]
    fn tag_selects_highest_classification_for_mixed_data() {
        let mut tracker = DataTaintTracker::new();
        let label = tracker.tag("mixed_source", b"name: Alice, api_key=sk-secret");
        assert_eq!(label.classification, Classification::Credentials);
    }

    #[spec("SEQ-001")]
    #[test]
    fn tag_inherits_upstream_taint_from_another_agent() {
        let mut tracker = DataTaintTracker::new();
        let label = tracker.tag("a2a:agent-upstream", b"delegated data");
        assert_ne!(label.classification, Classification::Public);
    }

    // =========================================================================
    // SEQ-002: Propagate taint through data transformations
    // =========================================================================

    #[spec("SEQ-002")]
    #[test]
    fn propagate_inherits_taint_from_all_inputs() {
        let mut tracker = DataTaintTracker::new();
        let label_a = tracker.tag("source_a", b"internal doc");
        let label_b = tracker.tag("source_b", b"public info");
        let output = tracker.propagate(&[&label_a, &label_b], "merge");
        assert_eq!(output.provenance_chain.len(), 3);
    }

    #[spec("SEQ-002")]
    #[test]
    fn propagate_records_transformation_type_in_provenance() {
        let mut tracker = DataTaintTracker::new();
        let label = tracker.tag("api", b"raw data");
        let output = tracker.propagate(&[&label], "base64_encode");
        assert!(
            output
                .provenance_chain
                .iter()
                .any(|s| s.contains("base64_encode"))
        );
    }

    #[spec("SEQ-002")]
    #[test]
    fn propagate_preserves_taint_through_encoding() {
        let mut tracker = DataTaintTracker::new();
        let label = tracker.tag("db", b"secret=hunter2");
        let encoded = tracker.propagate(&[&label], "base64_encode");
        assert_eq!(encoded.classification, Classification::Credentials);
    }

    #[spec("SEQ-002")]
    #[test]
    fn propagate_selects_highest_classification_on_merge() {
        let mut tracker = DataTaintTracker::new();
        let public = tracker.tag("web", b"hello world");
        let secret = tracker.tag("vault", b"api_key=sk-123");
        let merged = tracker.propagate(&[&public, &secret], "concatenate");
        assert_eq!(merged.classification, Classification::Credentials);
    }

    #[spec("SEQ-002")]
    #[test]
    fn propagate_preserves_taint_through_summarization() {
        let mut tracker = DataTaintTracker::new();
        let label = tracker.tag("hr", b"employee SSN: 123-45-6789");
        let summary = tracker.propagate(&[&label], "summarize");
        assert_eq!(summary.classification, Classification::Pii);
    }

    #[spec("SEQ-002")]
    #[test]
    fn propagate_taints_llm_output_when_input_tainted() {
        let mut tracker = DataTaintTracker::new();
        let label = tracker.tag("db", b"internal config values");
        let llm_output = tracker.propagate(&[&label], "llm_inference");
        assert_ne!(llm_output.classification, Classification::Public);
    }
}
