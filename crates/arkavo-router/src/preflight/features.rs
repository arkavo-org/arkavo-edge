//! Pre-flight feature extraction for boolean circuit inputs

use once_cell::sync::Lazy;
use regex::Regex;

/// Boolean features extractable from raw request text
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PreflightFeature {
    /// SSN, credit card, email patterns
    InputContainsPII,
    /// Toxicity keywords (configurable)
    InputContainsProfanity,
    /// Contains triple backticks
    InputContainsCodeBlock,
    /// SELECT, DROP, INSERT, UPDATE, DELETE
    InputContainsSQLKeywords,
    /// rm, sudo, chmod, curl, wget
    InputContainsShellCommands,
    /// Character count exceeds threshold
    InputLengthExceedsThreshold(usize),
    /// Contains http:// or https://
    InputContainsURL,
    /// Contains base64 pattern
    InputContainsBase64,
    /// Custom regex pattern
    Custom(String),
}

// Pre-compiled regexes for performance
static SSN_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").expect("valid SSN regex"));

static CREDIT_CARD_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b\d{4}[- ]?\d{4}[- ]?\d{4}[- ]?\d{4}\b").expect("valid CC regex"));

static EMAIL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b").expect("valid email regex")
});

static URL_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"https?://[^\s]+").expect("valid URL regex"));

static BASE64_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[A-Za-z0-9+/]{20,}={0,2}").expect("valid base64 regex"));

static SQL_KEYWORDS: &[&str] = &[
    "SELECT", "DROP", "INSERT", "UPDATE", "DELETE", "TRUNCATE", "ALTER",
];
static SHELL_COMMANDS: &[&str] = &["rm ", "sudo ", "chmod ", "curl ", "wget ", "sh ", "bash "];

impl PreflightFeature {
    /// Extract boolean value from raw input text
    #[must_use]
    pub fn extract(&self, input: &str) -> bool {
        match self {
            Self::InputContainsPII => {
                SSN_REGEX.is_match(input)
                    || CREDIT_CARD_REGEX.is_match(input)
                    || EMAIL_REGEX.is_match(input)
            }
            Self::InputContainsProfanity => {
                // Placeholder - would use configurable word list
                false
            }
            Self::InputContainsCodeBlock => input.contains("```"),
            Self::InputContainsSQLKeywords => {
                let upper = input.to_uppercase();
                SQL_KEYWORDS.iter().any(|kw| upper.contains(kw))
            }
            Self::InputContainsShellCommands => {
                let lower = input.to_lowercase();
                SHELL_COMMANDS.iter().any(|cmd| lower.contains(cmd))
            }
            Self::InputLengthExceedsThreshold(limit) => input.len() > *limit,
            Self::InputContainsURL => URL_REGEX.is_match(input),
            Self::InputContainsBase64 => BASE64_REGEX.is_match(input),
            Self::Custom(pattern) => Regex::new(pattern)
                .map(|re| re.is_match(input))
                .unwrap_or(false),
        }
    }

    /// Get feature name for debugging
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::InputContainsPII => "InputContainsPII".into(),
            Self::InputContainsProfanity => "InputContainsProfanity".into(),
            Self::InputContainsCodeBlock => "InputContainsCodeBlock".into(),
            Self::InputContainsSQLKeywords => "InputContainsSQLKeywords".into(),
            Self::InputContainsShellCommands => "InputContainsShellCommands".into(),
            Self::InputLengthExceedsThreshold(n) => format!("InputLengthExceedsThreshold({n})"),
            Self::InputContainsURL => "InputContainsURL".into(),
            Self::InputContainsBase64 => "InputContainsBase64".into(),
            Self::Custom(p) => format!("Custom({p})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssn_detection() {
        let feature = PreflightFeature::InputContainsPII;
        assert!(feature.extract("My SSN is 123-45-6789"));
        assert!(!feature.extract("My number is 12345"));
    }

    #[test]
    fn test_credit_card_detection() {
        let feature = PreflightFeature::InputContainsPII;
        assert!(feature.extract("Card: 4111-1111-1111-1111"));
        assert!(feature.extract("Card: 4111111111111111"));
        assert!(!feature.extract("Card: 411111111111"));
    }

    #[test]
    fn test_email_detection() {
        let feature = PreflightFeature::InputContainsPII;
        assert!(feature.extract("Email me at test@example.com"));
        assert!(!feature.extract("Email me at test@"));
    }

    #[test]
    fn test_sql_keywords() {
        let feature = PreflightFeature::InputContainsSQLKeywords;
        assert!(feature.extract("DROP TABLE users;"));
        assert!(feature.extract("select * from users"));
        assert!(!feature.extract("What is the weather?"));
    }

    #[test]
    fn test_shell_commands() {
        let feature = PreflightFeature::InputContainsShellCommands;
        assert!(feature.extract("Run sudo rm -rf /"));
        assert!(feature.extract("Use curl to fetch"));
        assert!(!feature.extract("What is the weather?"));
    }

    #[test]
    fn test_code_block() {
        let feature = PreflightFeature::InputContainsCodeBlock;
        assert!(feature.extract("```rust\nfn main() {}\n```"));
        assert!(!feature.extract("No code here"));
    }

    #[test]
    fn test_length_threshold() {
        let feature = PreflightFeature::InputLengthExceedsThreshold(10);
        assert!(feature.extract("This is a long string"));
        assert!(!feature.extract("Short"));
    }

    #[test]
    fn test_url_detection() {
        let feature = PreflightFeature::InputContainsURL;
        assert!(feature.extract("Visit https://example.com"));
        assert!(feature.extract("Check http://test.org/path"));
        assert!(!feature.extract("No URL here"));
    }
}
