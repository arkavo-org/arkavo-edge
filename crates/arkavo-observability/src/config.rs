use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info, instrument, warn};

/// Secure configuration provider that validates and injects configuration
/// without directly accessing potentially unsafe environment variables
#[derive(Debug, Clone)]
pub struct SecureConfigProvider {
    /// Runtime configuration values that have been validated
    config_map: HashMap<String, String>,
    /// Whether to allow fallback to environment variables (controlled)
    allow_env_fallback: bool,
    /// Validation rules for configuration keys
    validators: HashMap<String, ConfigValidator>,
}

/// Configuration validation rules
#[derive(Debug, Clone, Default)]
pub struct ConfigValidator {
    /// Whether this config value is required
    pub required: bool,
    /// Pattern the value must match (optional)
    pub pattern: Option<String>,
    /// Minimum length for string values
    pub min_length: Option<usize>,
    /// Maximum length for string values  
    pub max_length: Option<usize>,
    /// Whether this value contains sensitive data
    pub sensitive: bool,
    /// Default value if not provided
    pub default_value: Option<String>,
}

impl ConfigValidator {
    pub fn required() -> Self {
        Self {
            required: true,
            ..Default::default()
        }
    }

    pub fn sensitive() -> Self {
        Self {
            sensitive: true,
            ..Default::default()
        }
    }

    pub fn with_default(default: impl Into<String>) -> Self {
        Self {
            default_value: Some(default.into()),
            ..Default::default()
        }
    }

    pub fn as_required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn as_sensitive(mut self) -> Self {
        self.sensitive = true;
        self
    }

    pub fn with_length_limits(mut self, min: usize, max: usize) -> Self {
        self.min_length = Some(min);
        self.max_length = Some(max);
        self
    }

    pub fn with_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.pattern = Some(pattern.into());
        self
    }
}

/// Configuration file format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigFile {
    /// Application-specific configuration sections
    pub sections: HashMap<String, HashMap<String, String>>,
}

impl SecureConfigProvider {
    /// Create a new secure config provider
    pub fn new() -> Self {
        Self {
            config_map: HashMap::new(),
            allow_env_fallback: false,
            validators: HashMap::new(),
        }
    }

    /// Enable controlled environment variable fallback (use with caution)
    pub fn with_env_fallback(mut self, allowed: bool) -> Self {
        self.allow_env_fallback = allowed;
        if allowed {
            warn!(
                "Environment variable fallback enabled - ensure this is intended for the deployment"
            );
        }
        self
    }

    /// Add a configuration value with validation
    #[instrument(skip(self, value), fields(key, sensitive = validator.sensitive))]
    pub fn set_config(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
        validator: ConfigValidator,
    ) -> Result<()> {
        let key = key.into();
        let value = value.into();

        // Validate the value
        self.validate_value(&key, &value, &validator)?;

        // Store the validator for future lookups
        self.validators.insert(key.clone(), validator.clone());

        // Store the config value
        self.config_map.insert(key.clone(), value);

        if validator.sensitive {
            info!(key = %key, "Sensitive configuration value set");
        } else {
            info!(key = %key, value = %"[set]", "Configuration value set");
        }

        Ok(())
    }

    /// Load configuration from a file
    #[instrument(skip(self, path))]
    pub fn load_from_file<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let config_file: ConfigFile = if path.extension().and_then(|s| s.to_str()) == Some("toml") {
            toml::from_str(&content)?
        } else {
            serde_json::from_str(&content)?
        };

        // Flatten sections into dot notation
        for (section, values) in config_file.sections {
            for (key, value) in values {
                let full_key = format!("{section}.{key}");

                // Use existing validator if available, otherwise create a default one
                let validator = self.validators.get(&full_key).cloned().unwrap_or_default();

                self.set_config(full_key, value, validator)?;
            }
        }

        info!(path = %path.display(), "Configuration loaded from file");
        Ok(())
    }

    /// Get a configuration value with validation
    #[instrument(skip(self), fields(key))]
    pub fn get_config(&self, key: &str) -> Result<Option<String>> {
        // First try direct config map
        if let Some(value) = self.config_map.get(key) {
            debug!(key = %key, "Configuration value found in direct map");
            return Ok(Some(value.clone()));
        }

        // Check if we have a validator with a default value
        if let Some(validator) = self.validators.get(key) {
            if let Some(default) = &validator.default_value {
                debug!(key = %key, "Using default configuration value");
                return Ok(Some(default.clone()));
            }

            // If required but not found, this is an error
            if validator.required {
                return Err(anyhow::anyhow!(
                    "Required configuration key '{}' not found",
                    key
                ));
            }
        }

        // Controlled environment variable fallback (if enabled)
        if self.allow_env_fallback {
            // Only allow specific, known-safe environment variables
            if self.is_safe_env_key(key)
                && let Ok(env_value) = std::env::var(key)
            {
                warn!(key = %key, "Using environment variable fallback");
                return Ok(Some(env_value));
            }
        }

        Ok(None)
    }

    /// Get a required configuration value
    pub fn get_required_config(&self, key: &str) -> Result<String> {
        self.get_config(key)?
            .ok_or_else(|| anyhow::anyhow!("Required configuration key '{}' not found", key))
    }

    /// Register a validator for a configuration key
    pub fn register_validator(&mut self, key: impl Into<String>, validator: ConfigValidator) {
        self.validators.insert(key.into(), validator);
    }

    /// Validate a configuration value against its validator
    fn validate_value(&self, key: &str, value: &str, validator: &ConfigValidator) -> Result<()> {
        // Check length constraints
        if let Some(min_length) = validator.min_length
            && value.len() < min_length
        {
            return Err(anyhow::anyhow!(
                "Configuration key '{}' value too short (minimum {} characters)",
                key,
                min_length
            ));
        }

        if let Some(max_length) = validator.max_length
            && value.len() > max_length
        {
            return Err(anyhow::anyhow!(
                "Configuration key '{}' value too long (maximum {} characters)",
                key,
                max_length
            ));
        }

        // Check pattern if specified
        if let Some(pattern) = &validator.pattern {
            let regex = regex::Regex::new(pattern)
                .with_context(|| format!("Invalid regex pattern for key '{key}'"))?;

            if !regex.is_match(value) {
                return Err(anyhow::anyhow!(
                    "Configuration key '{}' value does not match required pattern",
                    key
                ));
            }
        }

        Ok(())
    }

    /// Check if an environment variable key is considered safe for fallback
    fn is_safe_env_key(&self, key: &str) -> bool {
        // Allow only specific, known-safe environment variables
        // This is a allowlist approach for security
        const SAFE_ENV_KEYS: &[&str] = &[
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "A2A_DISCOVERY_METHOD",
            "A2A_SERVER_PORT",
            "A2A_TIMEOUT_MS",
            // Add other safe environment variables as needed
        ];

        SAFE_ENV_KEYS.contains(&key)
    }

    /// Get all configuration keys (for debugging)
    pub fn get_all_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.config_map.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// Create a masked view of the configuration for logging
    pub fn get_masked_config(&self) -> HashMap<String, String> {
        self.config_map
            .iter()
            .map(|(key, value)| {
                let masked_value = if self.validators.get(key).is_some_and(|v| v.sensitive) {
                    "[SENSITIVE]".to_string()
                } else {
                    value.clone()
                };
                (key.clone(), masked_value)
            })
            .collect()
    }
}

impl Default for SecureConfigProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for common configuration patterns
pub struct SecureConfigBuilder {
    provider: SecureConfigProvider,
}

impl SecureConfigBuilder {
    pub fn new() -> Self {
        Self {
            provider: SecureConfigProvider::new(),
        }
    }

    /// Add OpenTelemetry configuration
    pub fn with_otel_config(mut self, endpoint: Option<String>) -> Self {
        self.provider.register_validator(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            ConfigValidator::with_default("http://localhost:4317").with_pattern(r"^https?://.*"),
        );

        if let Some(endpoint) = endpoint {
            self.provider
                .set_config(
                    "OTEL_EXPORTER_OTLP_ENDPOINT",
                    endpoint,
                    ConfigValidator::default(),
                )
                .unwrap();
        }

        self
    }

    /// Add authentication configuration
    pub fn with_auth_config(mut self) -> Self {
        // Register sensitive auth validators
        self.provider.register_validator(
            "ARKAVO_MASTER_KEY",
            ConfigValidator::sensitive().with_length_limits(32, 128),
        );
        self.provider.register_validator(
            "ANTHROPIC_API_KEY",
            ConfigValidator::sensitive().with_pattern(r"^sk-ant-"),
        );
        self.provider.register_validator(
            "HF_TOKEN",
            ConfigValidator::sensitive().with_pattern(r"^hf_"),
        );

        self
    }

    /// Add LLM provider configuration
    pub fn with_llm_config(mut self) -> Self {
        self.provider.register_validator(
            "LLM_PROVIDER",
            ConfigValidator::with_default("ollama").with_pattern(r"^(ollama|openai|anthropic)$"),
        );
        self.provider.register_validator(
            "OLLAMA_BASE_URL",
            ConfigValidator::with_default("http://localhost:11434").with_pattern(r"^https?://.*"),
        );
        self.provider
            .register_validator("OLLAMA_MODEL", ConfigValidator::with_default("llama3.2:3b"));

        self
    }

    /// Enable environment fallback for deployment
    pub fn with_env_fallback(mut self) -> Self {
        self.provider = self.provider.with_env_fallback(true);
        self
    }

    pub fn build(self) -> SecureConfigProvider {
        self.provider
    }
}

impl Default for SecureConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_basic_config_operations() {
        let mut provider = SecureConfigProvider::new();

        // Set a simple config value
        provider
            .set_config("test.key", "test_value", ConfigValidator::default())
            .unwrap();

        // Retrieve the value
        let value = provider.get_config("test.key").unwrap();
        assert_eq!(value, Some("test_value".to_string()));

        // Non-existent key should return None
        let missing = provider.get_config("missing.key").unwrap();
        assert_eq!(missing, None);
    }

    #[test]
    fn test_required_config_validation() {
        let mut provider = SecureConfigProvider::new();

        // Register a required validator
        provider.register_validator("required.key", ConfigValidator::required());

        // Should fail when required key is missing
        let result = provider.get_required_config("required.key");
        assert!(result.is_err());

        // Should succeed when required key is provided
        provider
            .set_config(
                "required.key",
                "required_value",
                ConfigValidator::required(),
            )
            .unwrap();

        let value = provider.get_required_config("required.key").unwrap();
        assert_eq!(value, "required_value");
    }

    #[test]
    fn test_config_validation() {
        let mut provider = SecureConfigProvider::new();

        // Test length validation
        let validator = ConfigValidator::default().with_length_limits(5, 10);

        // Too short should fail
        let result = provider.set_config("test.key", "abc", validator.clone());
        assert!(result.is_err());

        // Too long should fail
        let result = provider.set_config("test.key", "this_is_too_long", validator.clone());
        assert!(result.is_err());

        // Just right should succeed
        let result = provider.set_config("test.key", "perfect", validator);
        assert!(result.is_ok());
    }

    #[test]
    fn test_default_values() {
        let mut provider = SecureConfigProvider::new();

        // Register a validator with a default value
        provider.register_validator(
            "default.key",
            ConfigValidator::with_default("default_value"),
        );

        // Should return default value when key is not set
        let value = provider.get_config("default.key").unwrap();
        assert_eq!(value, Some("default_value".to_string()));
    }

    #[test]
    fn test_sensitive_config_masking() {
        let mut provider = SecureConfigProvider::new();

        // Set a sensitive config value
        provider
            .set_config("secret.key", "secret_value", ConfigValidator::sensitive())
            .unwrap();

        // Set a non-sensitive config value
        provider
            .set_config("public.key", "public_value", ConfigValidator::default())
            .unwrap();

        let masked_config = provider.get_masked_config();

        assert_eq!(
            masked_config.get("secret.key"),
            Some(&"[SENSITIVE]".to_string())
        );
        assert_eq!(
            masked_config.get("public.key"),
            Some(&"public_value".to_string())
        );
    }

    #[test]
    fn test_config_file_loading() {
        let mut provider = SecureConfigProvider::new();

        // Create a temporary JSON config file instead of TOML
        let mut temp_file = NamedTempFile::with_suffix(".json").unwrap();
        write!(
            temp_file,
            r#"{{
    "sections": {{
        "database": {{
            "host": "localhost",
            "port": "5432"
        }},
        "auth": {{
            "secret_key": "test_secret"
        }}
    }}
}}"#
        )
        .unwrap();
        temp_file.flush().unwrap();

        // Load from file
        provider.load_from_file(temp_file.path()).unwrap();

        // Verify values were loaded
        assert_eq!(
            provider.get_config("database.host").unwrap(),
            Some("localhost".to_string())
        );
        assert_eq!(
            provider.get_config("database.port").unwrap(),
            Some("5432".to_string())
        );
    }

    #[test]
    fn test_secure_config_builder() {
        let provider = SecureConfigBuilder::new()
            .with_otel_config(Some("http://test:4317".to_string()))
            .with_auth_config()
            .with_llm_config()
            .build();

        // Should have OTEL endpoint set
        assert_eq!(
            provider.get_config("OTEL_EXPORTER_OTLP_ENDPOINT").unwrap(),
            Some("http://test:4317".to_string())
        );

        // Should have LLM provider default
        assert_eq!(
            provider.get_config("LLM_PROVIDER").unwrap(),
            Some("ollama".to_string())
        );
    }
}
