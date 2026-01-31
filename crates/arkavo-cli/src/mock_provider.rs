//! Mock API Provider for Security Testing
//!
//! This module provides a mock LLM provider for testing security controls:
//! - DLP (Data Loss Prevention) - prevents sensitive data exfiltration
//! - PII (Personally Identifiable Information) leak detection
//! - API key validation
//! - Request/response inspection
//!
//! Usage: Set ARKAVO_MOCK_PROVIDER=1 to enable mock mode

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Mock provider configuration
#[derive(Debug, Clone)]
pub struct MockProviderConfig {
    /// Whether to simulate API key validation
    pub validate_api_key: bool,
    /// Expected API key (for testing auth failures)
    pub expected_api_key: Option<String>,
    /// Whether to detect PII in prompts
    pub detect_pii: bool,
    /// Whether to detect DLP violations
    pub detect_dlp: bool,
    /// Simulated response delay (ms)
    pub response_delay_ms: u64,
    /// Custom responses for specific patterns
    pub custom_responses: HashMap<String, String>,
}

impl Default for MockProviderConfig {
    fn default() -> Self {
        Self {
            validate_api_key: true,
            expected_api_key: Some("test-api-key-12345".to_string()),
            detect_pii: true,
            detect_dlp: true,
            response_delay_ms: 100,
            custom_responses: HashMap::new(),
        }
    }
}

/// PII detection patterns
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiiType {
    Email,
    Phone,
    Ssn,
    CreditCard,
    ApiKey,
    Password,
    Token,
}

impl PiiType {
    /// Get regex pattern for this PII type
    pub fn pattern(&self) -> &'static str {
        match self {
            PiiType::Email => r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}",
            PiiType::Phone => r"\b\d{3}[-.]?\d{3}[-.]?\d{4}\b",
            PiiType::Ssn => r"\b\d{3}-\d{2}-\d{4}\b",
            PiiType::CreditCard => r"\b(?:\d{4}[- ]?){3}\d{4}\b",
            PiiType::ApiKey => r"\b(?:sk-|pk-|api[_-]?key|token[_-]?)[a-zA-Z0-9]{16,}\b",
            PiiType::Password => r"\b(?:password|passwd|pwd)\s*[=:]\s*\S+",
            PiiType::Token => {
                r"\b(?:bearer|token)\s+[a-zA-Z0-9_-]+\.[a-zA-Z0-9_-]+\.[a-zA-Z0-9_-]+\b"
            }
        }
    }

    /// Get human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            PiiType::Email => "email address",
            PiiType::Phone => "phone number",
            PiiType::Ssn => "social security number",
            PiiType::CreditCard => "credit card number",
            PiiType::ApiKey => "API key",
            PiiType::Password => "password",
            PiiType::Token => "authentication token",
        }
    }
}

/// DLP detection patterns for sensitive data categories
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DlpCategory {
    /// Financial data
    Financial,
    /// Healthcare data
    Healthcare,
    /// Credentials
    Credentials,
    /// Internal system data
    Internal,
    /// Proprietary code
    SourceCode,
}

impl DlpCategory {
    /// Get patterns for this DLP category
    pub fn patterns(&self) -> Vec<&'static str> {
        match self {
            DlpCategory::Financial => vec![
                r"\$\d{1,3}(,\d{3})*(\.\d{2})?", // Dollar amounts
                r"\b(?:visa|mastercard|amex|discover)\b",
                r"\b(?:bank|routing)\s*#?\s*\d+",
            ],
            DlpCategory::Healthcare => vec![
                r"\b(?:patient|medical|diagnosis|prescription)\b",
                r"\b(?:mrn|medical record)\s*#?\s*\d+",
                r"\b(?:hipaa|phi|pii)\b",
            ],
            DlpCategory::Credentials => vec![
                r"(?i)\b(?:aws_access_key_id|aws_secret_access_key)\b",
                r"(?i)\b(?:private[_-]?key|secret[_-]?key)\b",
                r"-----BEGIN (RSA |DSA |EC |OPENSSH )?PRIVATE KEY-----",
            ],
            DlpCategory::Internal => vec![
                r"\b(?:internal|confidential|proprietary)\b",
                r"\b(?:10\.\d{1,3}\.\d{1,3}\.\d{1,3}|192\.168\.\d{1,3}\.\d{1,3})\b",
            ],
            DlpCategory::SourceCode => vec![r"\b(?:TODO|FIXME|HACK|XXX)\s*[:\-]\s*\w+"],
        }
    }

    /// Get severity level
    pub fn severity(&self) -> &'static str {
        match self {
            DlpCategory::Credentials => "critical",
            DlpCategory::Financial => "high",
            DlpCategory::Healthcare => "high",
            DlpCategory::Internal => "medium",
            DlpCategory::SourceCode => "low",
        }
    }
}

/// Detected sensitive data finding
#[derive(Debug, Clone)]
pub struct SensitiveDataFinding {
    pub finding_type: String,
    pub category: String,
    pub severity: String,
    pub position: (usize, usize),
    pub redacted: String,
}

/// Mock LLM provider for security testing
pub struct MockProvider {
    config: MockProviderConfig,
    request_log: Arc<Mutex<Vec<MockRequest>>>,
    response_log: Arc<Mutex<Vec<MockResponse>>>,
}

/// Recorded request
#[derive(Debug, Clone)]
pub struct MockRequest {
    pub timestamp: std::time::SystemTime,
    pub api_key: Option<String>,
    pub prompt: String,
    pub model: String,
}

/// Recorded response
#[derive(Debug, Clone)]
pub struct MockResponse {
    pub timestamp: std::time::SystemTime,
    pub success: bool,
    pub blocked: bool,
    pub block_reason: Option<String>,
    pub content: String,
    pub findings: Vec<SensitiveDataFinding>,
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MockProvider {
    /// Create a new mock provider with default config
    pub fn new() -> Self {
        Self::with_config(MockProviderConfig::default())
    }

    /// Create a new mock provider with custom config
    pub fn with_config(config: MockProviderConfig) -> Self {
        Self {
            config,
            request_log: Arc::new(Mutex::new(Vec::new())),
            response_log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Check if mock provider should be used
    pub fn is_enabled() -> bool {
        std::env::var("ARKAVO_MOCK_PROVIDER")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false)
    }

    /// Send a chat completion request (mock)
    ///
    /// # Panics
    /// Panics if the internal request or response mutex is poisoned
    pub async fn chat_completion(
        &self,
        api_key: &str,
        model: &str,
        prompt: &str,
    ) -> Result<MockResponse, MockProviderError> {
        // Validate API key if configured
        if self.config.validate_api_key
            && let Some(expected) = &self.config.expected_api_key
            && api_key != expected
        {
            return Err(MockProviderError::InvalidApiKey);
        }

        // Log the request
        let request = MockRequest {
            timestamp: std::time::SystemTime::now(),
            api_key: Some(api_key.to_string()),
            prompt: prompt.to_string(),
            model: model.to_string(),
        };
        self.request_log.lock().unwrap().push(request);

        // Check for PII
        let mut findings = Vec::new();
        if self.config.detect_pii {
            findings.extend(self.detect_pii(prompt));
        }

        // Check for DLP violations
        if self.config.detect_dlp {
            findings.extend(self.detect_dlp(prompt));
        }

        // Simulate response delay
        if self.config.response_delay_ms > 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(
                self.config.response_delay_ms,
            ))
            .await;
        }

        // Check for custom responses
        for (pattern, response) in &self.config.custom_responses {
            if prompt.contains(pattern) {
                let mock_response = MockResponse {
                    timestamp: std::time::SystemTime::now(),
                    success: true,
                    blocked: false,
                    block_reason: None,
                    content: response.clone(),
                    findings: findings.clone(),
                };
                self.response_log
                    .lock()
                    .unwrap()
                    .push(mock_response.clone());
                return Ok(mock_response);
            }
        }

        // Block if sensitive data detected
        let has_critical = findings.iter().any(|f| f.severity == "critical");
        let has_high = findings.iter().any(|f| f.severity == "high");

        if has_critical || has_high {
            let block_reason = if has_critical {
                "Request blocked: critical sensitive data detected"
            } else {
                "Request blocked: high-risk data detected"
            };

            let response = MockResponse {
                timestamp: std::time::SystemTime::now(),
                success: false,
                blocked: true,
                block_reason: Some(block_reason.to_string()),
                content: format!("[BLOCKED] {block_reason}"),
                findings,
            };
            self.response_log.lock().unwrap().push(response.clone());
            return Ok(response);
        }

        // Generate mock response
        let content = self.generate_mock_response(prompt);

        let response = MockResponse {
            timestamp: std::time::SystemTime::now(),
            success: true,
            blocked: false,
            block_reason: None,
            content,
            findings,
        };

        self.response_log.lock().unwrap().push(response.clone());
        Ok(response)
    }

    /// Detect PII in text
    fn detect_pii(&self, text: &str) -> Vec<SensitiveDataFinding> {
        let mut findings = Vec::new();

        for pii_type in [
            PiiType::Email,
            PiiType::Phone,
            PiiType::Ssn,
            PiiType::CreditCard,
            PiiType::ApiKey,
            PiiType::Password,
            PiiType::Token,
        ] {
            if let Ok(re) = regex::Regex::new(pii_type.pattern()) {
                for mat in re.find_iter(text) {
                    findings.push(SensitiveDataFinding {
                        finding_type: pii_type.name().to_string(),
                        category: "PII".to_string(),
                        severity: "high".to_string(),
                        position: (mat.start(), mat.end()),
                        redacted: format!("[REDACTED {}]", pii_type.name().to_uppercase()),
                    });
                }
            }
        }

        findings
    }

    /// Detect DLP violations in text
    fn detect_dlp(&self, text: &str) -> Vec<SensitiveDataFinding> {
        let mut findings = Vec::new();

        for category in [
            DlpCategory::Financial,
            DlpCategory::Healthcare,
            DlpCategory::Credentials,
            DlpCategory::Internal,
            DlpCategory::SourceCode,
        ] {
            for pattern in category.patterns() {
                if let Ok(re) = regex::Regex::new(pattern) {
                    for mat in re.find_iter(text) {
                        findings.push(SensitiveDataFinding {
                            finding_type: format!("{category:?}"),
                            category: "DLP".to_string(),
                            severity: category.severity().to_string(),
                            position: (mat.start(), mat.end()),
                            redacted: "[REDACTED]".to_string(),
                        });
                    }
                }
            }
        }

        findings
    }

    /// Generate a mock response
    fn generate_mock_response(&self, prompt: &str) -> String {
        // Simple response generation based on prompt content
        if prompt.to_lowercase().contains("hello") || prompt.to_lowercase().contains("hi") {
            "Hello! I'm a mock AI assistant for testing purposes. How can I help you today?"
                .to_string()
        } else if prompt.to_lowercase().contains("security") {
            "Security is important! This mock provider helps test DLP and PII detection."
                .to_string()
        } else if prompt.to_lowercase().contains("api key")
            || prompt.to_lowercase().contains("token")
        {
            "I cannot provide or validate API keys. Please use secure credential management."
                .to_string()
        } else {
            format!(
                "Mock response: Received prompt of {} characters. [Security testing mode active]",
                prompt.len()
            )
        }
    }

    /// Get request log
    ///
    /// # Panics
    /// Panics if the internal request mutex is poisoned
    pub fn get_request_log(&self) -> Vec<MockRequest> {
        self.request_log.lock().unwrap().clone()
    }

    /// Get response log
    ///
    /// # Panics
    /// Panics if the internal response mutex is poisoned
    pub fn get_response_log(&self) -> Vec<MockResponse> {
        self.response_log.lock().unwrap().clone()
    }

    /// Clear logs
    ///
    /// # Panics
    /// Panics if the internal request or response mutex is poisoned
    pub fn clear_logs(&self) {
        self.request_log.lock().unwrap().clear();
        self.response_log.lock().unwrap().clear();
    }
}

/// Mock provider errors
#[derive(Debug)]
pub enum MockProviderError {
    InvalidApiKey,
    DlpViolation,
    PiiDetected,
    RateLimitExceeded,
    NetworkError,
}

impl std::fmt::Display for MockProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MockProviderError::InvalidApiKey => write!(f, "Invalid API key"),
            MockProviderError::DlpViolation => write!(f, "Request blocked by DLP policy"),
            MockProviderError::PiiDetected => write!(f, "PII detected in request"),
            MockProviderError::RateLimitExceeded => write!(f, "Rate limit exceeded"),
            MockProviderError::NetworkError => write!(f, "Network error (mock)"),
        }
    }
}

impl std::error::Error for MockProviderError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pii_detection_email() {
        let provider = MockProvider::new();
        let text = "Contact me at john.doe@example.com for details";
        let findings = provider.detect_pii(text);

        assert!(!findings.is_empty());
        assert!(findings.iter().any(|f| f.finding_type == "email address"));
    }

    #[test]
    fn test_pii_detection_api_key() {
        let provider = MockProvider::new();
        let text = "My API key is sk-abc123xyz789secretkey123";
        let findings = provider.detect_pii(text);

        assert!(!findings.is_empty());
        assert!(findings.iter().any(|f| f.finding_type == "API key"));
    }

    #[test]
    fn test_dlp_detection_credentials() {
        let provider = MockProvider::new();
        let text = "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
        let findings = provider.detect_dlp(text);

        // The pattern looks for aws_secret_access_key keyword
        assert!(
            findings
                .iter()
                .any(|f| f.severity == "critical" || f.finding_type.contains("Credentials"))
        );
    }

    #[test]
    fn test_no_pii_in_safe_text() {
        let provider = MockProvider::new();
        let text = "Hello, how are you today? The weather is nice.";
        let findings = provider.detect_pii(text);

        assert!(findings.is_empty());
    }
}
