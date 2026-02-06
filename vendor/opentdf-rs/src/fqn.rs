//! Fully Qualified Name (FQN) support for OpenTDF attributes
//!
//! This module provides URL-based FQN parsing and validation for attribute identifiers,
//! following the OpenTDF standard format:
//!
//! - Namespace: `https://<namespace>`
//! - Attribute: `https://<namespace>/attr/<name>`
//! - Value: `https://<namespace>/attr/<name>/value/<value>`
//!
//! # Example
//!
//! ```
//! use opentdf::fqn::AttributeFqn;
//!
//! let fqn = AttributeFqn::parse("https://example.com/attr/classification/value/secret")?;
//! assert_eq!(fqn.get_namespace(), "example.com");
//! assert_eq!(fqn.get_name(), "classification");
//! assert_eq!(fqn.get_value(), Some("secret"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::policy::{AttributeIdentifier, AttributeValue, FqnError};
use std::collections::HashMap;

/// Fully Qualified Name for an OpenTDF attribute
///
/// FQNs follow the format: `https://<namespace>/attr/<name>/value/<value>`
/// where the `/value/<value>` part is optional for attribute definitions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AttributeFqn {
    namespace: String,
    name: String,
    value: Option<String>,
}

impl AttributeFqn {
    /// Parse an FQN string with strict validation
    ///
    /// # Validation Rules
    /// - Must use HTTPS scheme (http:// is rejected)
    /// - Must follow structure: `https://<namespace>/attr/<name>[/value/<value>]`
    /// - Namespace must be a valid domain-like identifier
    /// - Name and value are percent-decoded
    ///
    /// # Example
    ///
    /// ```
    /// use opentdf::fqn::AttributeFqn;
    ///
    /// let fqn = AttributeFqn::parse("https://example.com/attr/classification/value/secret")?;
    /// assert_eq!(fqn.get_namespace(), "example.com");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse(s: &str) -> Result<Self, FqnError> {
        Self::parse_with_rules(s, &FqnValidationRules::default())
    }

    /// Parse an FQN with custom validation rules
    pub fn parse_with_rules(s: &str, rules: &FqnValidationRules) -> Result<Self, FqnError> {
        // Check for HTTPS scheme
        if rules.require_https && s.starts_with("http://") {
            return Err(FqnError::NotHttps { url: s.to_string() });
        }

        // Extract scheme
        let (scheme, rest) = s
            .split_once("://")
            .ok_or_else(|| FqnError::MalformedUrl("Missing scheme separator ://".to_string()))?;

        if scheme != "https" && scheme != "http" {
            return Err(FqnError::InvalidScheme {
                expected: "https",
                found: scheme.to_string(),
            });
        }

        // Parse namespace (everything before first /)
        let (namespace, path) = match rest.split_once('/') {
            Some((ns, p)) => (ns, p),
            None => {
                // Just a namespace, no path
                return Ok(Self {
                    namespace: rest.to_lowercase(),
                    name: String::new(),
                    value: None,
                });
            }
        };

        // Validate namespace
        if namespace.is_empty() {
            return Err(FqnError::MissingComponent {
                component: "namespace",
            });
        }

        let namespace = namespace.to_lowercase(); // Case-insensitive

        // If we require /attr/ structure, validate it
        if rules.require_attr_structure {
            if !path.starts_with("attr/") {
                return Err(FqnError::MissingAttrStructure { url: s.to_string() });
            }

            // Parse: attr/<name>[/value/<value>]
            let path = &path[5..]; // Skip "attr/"

            if path.is_empty() {
                return Err(FqnError::MissingComponent { component: "name" });
            }

            // Split remaining path
            if let Some((name, value_part)) = path.split_once("/value/") {
                // Has value component
                Ok(Self {
                    namespace,
                    name: percent_decode(name),
                    value: Some(percent_decode(value_part)),
                })
            } else {
                // Just attribute name, no value
                Ok(Self {
                    namespace,
                    name: percent_decode(path),
                    value: None,
                })
            }
        } else {
            // Lenient parsing - just split on /
            let parts: Vec<&str> = path.split('/').collect();
            match parts.as_slice() {
                [] => Ok(Self {
                    namespace,
                    name: String::new(),
                    value: None,
                }),
                [name] => Ok(Self {
                    namespace,
                    name: percent_decode(name),
                    value: None,
                }),
                [name, value] => Ok(Self {
                    namespace,
                    name: percent_decode(name),
                    value: Some(percent_decode(value)),
                }),
                _ => Ok(Self {
                    namespace,
                    name: percent_decode(parts[0]),
                    value: Some(percent_decode(&parts[1..].join("/"))),
                }),
            }
        }
    }

    /// Create an FQN for a namespace only
    pub fn namespace(ns: &str) -> Self {
        Self {
            namespace: ns.to_lowercase(),
            name: String::new(),
            value: None,
        }
    }

    /// Create an FQN for an attribute (namespace + name)
    pub fn attribute(ns: &str, name: &str) -> Self {
        Self {
            namespace: ns.to_lowercase(),
            name: name.to_string(),
            value: None,
        }
    }

    /// Create an FQN for an attribute value (namespace + name + value)
    pub fn with_value(ns: &str, name: &str, value: &str) -> Self {
        Self {
            namespace: ns.to_lowercase(),
            name: name.to_string(),
            value: Some(value.to_string()),
        }
    }

    /// Get the namespace portion
    pub fn get_namespace(&self) -> &str {
        &self.namespace
    }

    /// Get the attribute name
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// Get the attribute value, if present
    pub fn get_value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// Convert to a full HTTPS URL string
    pub fn to_url(&self) -> String {
        if self.name.is_empty() {
            format!("https://{}", self.namespace)
        } else if let Some(value) = &self.value {
            format!(
                "https://{}/attr/{}/value/{}",
                self.namespace,
                percent_encode(&self.name),
                percent_encode(value)
            )
        } else {
            format!(
                "https://{}/attr/{}",
                self.namespace,
                percent_encode(&self.name)
            )
        }
    }

    /// Convert to an AttributeIdentifier (namespace:name format)
    pub fn to_identifier(&self) -> AttributeIdentifier {
        AttributeIdentifier::new(&self.namespace, &self.name)
    }

    /// Convert value to AttributeValue if present
    pub fn to_attribute_value(&self) -> Option<AttributeValue> {
        self.value
            .as_ref()
            .map(|v| AttributeValue::String(v.clone()))
    }

    /// Validate against a namespace registry
    pub fn validate_namespace(&self, registry: &NamespaceRegistry) -> Result<(), FqnError> {
        if registry.is_registered(&self.namespace) {
            Ok(())
        } else {
            Err(FqnError::NamespaceNotRegistered {
                namespace: self.namespace.clone(),
            })
        }
    }
}

impl std::fmt::Display for AttributeFqn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_url())
    }
}

impl std::str::FromStr for AttributeFqn {
    type Err = FqnError;

    /// Parse an FQN from a string using default validation rules
    ///
    /// # Example
    /// ```
    /// use opentdf::fqn::AttributeFqn;
    /// use std::str::FromStr;
    ///
    /// let fqn = AttributeFqn::from_str("https://example.com/attr/clearance/value/secret")?;
    /// assert_eq!(fqn.get_namespace(), "example.com");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl<'a> TryFrom<&'a str> for AttributeFqn {
    type Error = FqnError;

    /// Convert from a string slice using default validation rules
    ///
    /// # Example
    /// ```
    /// use opentdf::fqn::AttributeFqn;
    ///
    /// let fqn: AttributeFqn = "https://example.com/attr/name/value/val".try_into()?;
    /// assert_eq!(fqn.get_name(), "name");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    fn try_from(s: &'a str) -> Result<Self, Self::Error> {
        Self::parse(s)
    }
}

/// FQN validation rules
#[derive(Debug, Clone)]
pub struct FqnValidationRules {
    /// Require HTTPS scheme (reject http://)
    pub require_https: bool,
    /// Require /attr/<name>/value/<value> structure
    pub require_attr_structure: bool,
    /// Validate namespace against registry
    pub validate_namespace_registry: bool,
}

impl Default for FqnValidationRules {
    fn default() -> Self {
        Self {
            require_https: true,
            require_attr_structure: true,
            validate_namespace_registry: false,
        }
    }
}

/// Registry of known namespaces for validation
#[derive(Debug, Clone)]
pub struct NamespaceRegistry {
    namespaces: HashMap<String, NamespaceInfo>,
}

#[derive(Debug, Clone)]
pub struct NamespaceInfo {
    pub name: String,
    pub description: Option<String>,
}

impl NamespaceRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            namespaces: HashMap::new(),
        }
    }

    /// Register a namespace
    pub fn register(&mut self, namespace: &str) -> &mut Self {
        self.namespaces.insert(
            namespace.to_lowercase(),
            NamespaceInfo {
                name: namespace.to_string(),
                description: None,
            },
        );
        self
    }

    /// Register a namespace with description
    pub fn register_with_description(&mut self, namespace: &str, description: &str) -> &mut Self {
        self.namespaces.insert(
            namespace.to_lowercase(),
            NamespaceInfo {
                name: namespace.to_string(),
                description: Some(description.to_string()),
            },
        );
        self
    }

    /// Check if a namespace is registered
    pub fn is_registered(&self, namespace: &str) -> bool {
        self.namespaces.contains_key(&namespace.to_lowercase())
    }

    /// Get namespace info
    pub fn get(&self, namespace: &str) -> Option<&NamespaceInfo> {
        self.namespaces.get(&namespace.to_lowercase())
    }
}

impl Default for NamespaceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple percent-encoding for URL paths
fn percent_encode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}

/// Simple percent-decoding for URL paths
fn percent_decode(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();

    while let Some(c) = chars.next() {
        if c == '%' {
            // Try to read two hex digits
            let hex: String = chars.by_ref().take(2).collect();
            if hex.len() == 2
                && let Ok(byte) = u8::from_str_radix(&hex, 16)
            {
                result.push(byte as char);
                continue;
            }
            // If decoding fails, keep the original
            result.push('%');
            result.push_str(&hex);
        } else {
            result.push(c);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_fqn() {
        let fqn =
            AttributeFqn::parse("https://example.com/attr/classification/value/secret").unwrap();
        assert_eq!(fqn.get_namespace(), "example.com");
        assert_eq!(fqn.get_name(), "classification");
        assert_eq!(fqn.get_value(), Some("secret"));
    }

    #[test]
    fn test_parse_attribute_only() {
        let fqn = AttributeFqn::parse("https://example.com/attr/classification").unwrap();
        assert_eq!(fqn.get_namespace(), "example.com");
        assert_eq!(fqn.get_name(), "classification");
        assert_eq!(fqn.get_value(), None);
    }

    #[test]
    fn test_parse_namespace_only() {
        let fqn = AttributeFqn::parse("https://example.com").unwrap();
        assert_eq!(fqn.get_namespace(), "example.com");
        assert_eq!(fqn.get_name(), "");
        assert_eq!(fqn.get_value(), None);
    }

    #[test]
    fn test_reject_http() {
        let result = AttributeFqn::parse("http://example.com/attr/test/value/val");
        assert!(matches!(result, Err(FqnError::NotHttps { .. })));
    }

    #[test]
    fn test_missing_attr_structure() {
        let result = AttributeFqn::parse("https://example.com/test/value");
        assert!(matches!(result, Err(FqnError::MissingAttrStructure { .. })));
    }

    #[test]
    fn test_case_insensitive_namespace() {
        let fqn1 = AttributeFqn::parse("https://EXAMPLE.COM/attr/test").unwrap();
        let fqn2 = AttributeFqn::parse("https://example.com/attr/test").unwrap();
        assert_eq!(fqn1.get_namespace(), fqn2.get_namespace());
    }

    #[test]
    fn test_percent_encoding() {
        let fqn = AttributeFqn::with_value("example.com", "test name", "test value");
        let url = fqn.to_url();
        assert!(url.contains("test%20name"));
        assert!(url.contains("test%20value"));
    }

    #[test]
    fn test_namespace_registry() {
        let mut registry = NamespaceRegistry::new();
        registry.register("example.com");
        registry.register("test.org");

        let fqn = AttributeFqn::parse("https://example.com/attr/test").unwrap();
        assert!(fqn.validate_namespace(&registry).is_ok());

        let fqn2 = AttributeFqn::parse("https://unknown.com/attr/test").unwrap();
        assert!(fqn2.validate_namespace(&registry).is_err());
    }

    #[test]
    fn test_to_identifier() {
        let fqn = AttributeFqn::parse("https://example.com/attr/classification").unwrap();
        let id = fqn.to_identifier();
        assert_eq!(id.as_string(), "example.com:classification");
    }
}
