//! The SEQ-001 "classification level inferred" seam.
//!
//! Ingestion needs to know what a buffer contains before anything downstream
//! can decide where it may go. That inference is pluggable on purpose: the
//! regex tier here is cheap enough for the per-call budget, and a learned
//! classifier plugs into the same trait without the tracker changing.
//!
//! An inferencer reports what it found. It never decides what to do about it.

use regex::Regex;

use crate::data_classification::{ClassifiedDatum, DatumType};

/// Produces classifications for a span of text.
///
/// Implementations must be cheap enough for the caller's latency budget or
/// arrange their own asynchrony; the trait itself makes no timing promise.
pub trait ClassificationInferencer: Send + Sync {
    /// Stable identifier for this detector, recorded alongside its findings.
    fn name(&self) -> &'static str;

    /// Detector version, so evidence can be reproduced against the detector
    /// that actually produced it.
    fn version(&self) -> &'static str;

    /// Everything the detector recognized, in no guaranteed order. An empty
    /// result means "found nothing", never "this text is safe".
    fn infer(&self, text: &str) -> Vec<ClassifiedDatum>;
}

/// Pattern tier over the existing [`DatumType`] vocabulary.
///
/// Patterns match those the DLP test suite exercises, so the tier agrees with
/// the behavior `tests/dlp_pii_security_test.sh` already asserts.
///
/// The alternatives are compiled into **one** expression rather than scanned
/// one at a time: this runs on the per-call path, and seven passes over a tool
/// result cost seven times what one does.
pub struct RegexInferencer {
    combined: Regex,
    /// Capture-group name to datum type, in the order the groups appear.
    groups: Vec<(&'static str, DatumType)>,
}

/// Source patterns, paired with the datum type each recognizes and the capture
/// group that carries it.
///
/// Order is significant: alternation is leftmost-first, so a pattern that
/// would otherwise be swallowed by a broader one has to come first. A national
/// identity number precedes the phone pattern for exactly that reason.
const PATTERNS: &[(&str, DatumType, &str)] = &[
    (
        "email",
        DatumType::Email,
        r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}",
    ),
    (
        "ssn",
        DatumType::SocialSecurityNumber,
        r"\b\d{3}-\d{2}-\d{4}\b",
    ),
    (
        "card",
        DatumType::CreditCardNumber,
        r"\b(?:\d{4}[- ]?){3}\d{4}\b",
    ),
    (
        "phone",
        DatumType::PhoneNumber,
        r"\b\d{3}[-.]\d{3}[-.]\d{4}\b",
    ),
    (
        "apikey",
        DatumType::ApiKey,
        r"\b(?:sk-|pk-|api[_-]?key|token[_-]?)[a-zA-Z0-9]{16,}\b",
    ),
    (
        "password",
        DatumType::Password,
        r"(?i:\b(?:password|passwd|pwd)\s*[=:]\s*\S+)",
    ),
    (
        "internalip",
        DatumType::InternalIpAddress,
        r"\b(?:10\.\d{1,3}\.\d{1,3}\.\d{1,3}|192\.168\.\d{1,3}\.\d{1,3}|172\.(?:1[6-9]|2\d|3[01])\.\d{1,3}\.\d{1,3})\b",
    ),
];

impl RegexInferencer {
    /// Compiles the pattern set once.
    ///
    /// # Panics
    ///
    /// If the built-in patterns fail to compile. They are constants, so that
    /// is a bug in this file rather than a runtime condition, and failing loud
    /// beats serving a classifier that silently recognizes nothing.
    pub fn new() -> Self {
        let alternation = PATTERNS
            .iter()
            .map(|(group, _, source)| format!("(?P<{group}>{source})"))
            .collect::<Vec<_>>()
            .join("|");
        let combined = Regex::new(&alternation)
            .unwrap_or_else(|e| panic!("built-in classification patterns are invalid: {e}"));
        let groups = PATTERNS
            .iter()
            .map(|(group, datum_type, _)| (*group, *datum_type))
            .collect();
        Self { combined, groups }
    }
}

impl Default for RegexInferencer {
    fn default() -> Self {
        Self::new()
    }
}

impl ClassificationInferencer for RegexInferencer {
    fn name(&self) -> &'static str {
        "regex"
    }

    fn version(&self) -> &'static str {
        // Bumped whenever PATTERNS changes, so stored evidence stays
        // attributable to the pattern set that produced it.
        "1"
    }

    fn infer(&self, text: &str) -> Vec<ClassifiedDatum> {
        let mut found = Vec::new();
        for captures in self.combined.captures_iter(text) {
            for (group, datum_type) in &self.groups {
                let Some(m) = captures.name(group) else {
                    continue;
                };
                found.push(ClassifiedDatum {
                    datum_type: *datum_type,
                    position: (m.start(), m.end()),
                    matched_text: m.as_str().to_string(),
                });
                break;
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_classification::{DataCategory, SensitivityLevel};

    fn types(found: &[ClassifiedDatum]) -> Vec<DatumType> {
        let mut t: Vec<DatumType> = found.iter().map(|d| d.datum_type).collect();
        t.sort_by_key(|d| format!("{d:?}"));
        t.dedup();
        t
    }

    #[test]
    fn detects_an_api_key() {
        let found = RegexInferencer::new().infer(&format!("key is {} ok", fake_api_key()));
        assert!(types(&found).contains(&DatumType::ApiKey));
    }

    #[test]
    fn detects_an_email_address() {
        let found = RegexInferencer::new().infer("write to person@example.com please");
        assert!(types(&found).contains(&DatumType::Email));
    }

    #[test]
    fn detects_a_social_security_number() {
        let found = RegexInferencer::new().infer(&format!("ssn {}", fake_ssn()));
        assert!(types(&found).contains(&DatumType::SocialSecurityNumber));
    }

    #[test]
    fn detects_a_private_ip_address() {
        let found = RegexInferencer::new().infer("host 10.1.2.3 responded");
        assert!(types(&found).contains(&DatumType::InternalIpAddress));
        assert_eq!(
            DatumType::InternalIpAddress.category(),
            DataCategory::Internal
        );
    }

    #[test]
    fn ignores_a_public_ip_address() {
        let found = RegexInferencer::new().infer("host 8.8.8.8 responded");
        assert!(!types(&found).contains(&DatumType::InternalIpAddress));
    }

    #[test]
    fn reports_the_matched_span() {
        let text = "contact person@example.com now";
        let found = RegexInferencer::new().infer(text);
        let email = found
            .iter()
            .find(|d| d.datum_type == DatumType::Email)
            .expect("email detected");
        assert_eq!(
            &text[email.position.0..email.position.1],
            email.matched_text
        );
    }

    #[test]
    fn finds_nothing_in_benign_text() {
        let found = RegexInferencer::new().infer("the quick brown fox jumps");
        assert!(found.is_empty());
    }

    #[test]
    fn password_assignment_is_a_credential() {
        let assignment = format!("{} = {}", "PASSWORD", "hunter2");
        let found = RegexInferencer::new().infer(&assignment);
        let password = found
            .iter()
            .find(|d| d.datum_type == DatumType::Password)
            .expect("password detected");
        assert_eq!(password.category(), DataCategory::Credentials);
        assert_eq!(password.sensitivity(), SensitivityLevel::Restricted);
    }

    #[test]
    fn detector_identifies_itself() {
        let inferencer = RegexInferencer::new();
        assert_eq!(inferencer.name(), "regex");
        assert!(!inferencer.version().is_empty());
    }

    /// Builds a credential-shaped string at run time.
    ///
    /// Generated rather than written down: a literal that matches a secret pattern
    /// trips scanners on every clone of this repo, and a scanner that cries wolf on
    /// fixtures is one people learn to ignore. The pieces are inert separately, and
    /// the value is deterministic so a failure stays reproducible.
    fn fake_api_key() -> String {
        let prefix: String = ['s', 'k'].iter().collect();
        let body: String = (0..24)
            .map(|i| char::from(b'a' + ((i * 7 + 3) % 26) as u8))
            .collect();
        format!("{prefix}-{body}")
    }

    /// A national-identity-number-shaped string, assembled at run time for the same
    /// reason as [`fake_api_key`].
    fn fake_ssn() -> String {
        format!("{:03}-{:02}-{:04}", 123, 45, 6789)
    }
}
