use crate::error::{DeepSeekError, Result};
use jsonschema::Draft;
use serde_json::Value;

/// Validates a JSON Schema for strict mode compatibility
pub fn validate_strict_schema(schema: &Value) -> Result<()> {
    // Check if it's an object schema
    if let Some(schema_type) = schema.get("type")
        && schema_type != "object"
    {
        return Ok(()); // Non-object schemas don't have strict requirements
    }

    // For object schemas, validate strict mode requirements
    validate_object_schema(schema)
}

fn validate_object_schema(schema: &Value) -> Result<()> {
    // Check for required array
    let properties = schema.get("properties").and_then(|p| p.as_object());
    let required = schema.get("required").and_then(|r| r.as_array());

    if let Some(props) = properties {
        if let Some(req_array) = required {
            // All properties must be in required array
            let required_set: std::collections::HashSet<String> = req_array
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();

            for key in props.keys() {
                if !required_set.contains(key) {
                    return Err(DeepSeekError::SchemaError {
                        message: format!(
                            "Property '{key}' must be listed in 'required' array for strict mode"
                        ),
                        details: Some(schema.clone()),
                    });
                }
            }
        } else if !props.is_empty() {
            return Err(DeepSeekError::SchemaError {
                message: "Object schema must have 'required' array listing all properties"
                    .to_string(),
                details: Some(schema.clone()),
            });
        }
    }

    // Check for additionalProperties: false
    match schema.get("additionalProperties") {
        Some(Value::Bool(false)) => {}
        Some(_) | None => {
            return Err(DeepSeekError::SchemaError {
                message: "Object schema must have 'additionalProperties: false' for strict mode"
                    .to_string(),
                details: Some(schema.clone()),
            });
        }
    }

    // Recursively validate nested schemas
    if let Some(props) = properties {
        for (prop_name, prop_schema) in props {
            validate_nested_schema(prop_schema).map_err(|e| {
                if let DeepSeekError::SchemaError { message, .. } = e {
                    DeepSeekError::SchemaError {
                        message: format!("In property '{prop_name}': {message}"),
                        details: Some(schema.clone()),
                    }
                } else {
                    e
                }
            })?;
        }
    }

    Ok(())
}

fn validate_nested_schema(schema: &Value) -> Result<()> {
    let schema_type = schema.get("type").and_then(|t| t.as_str());

    match schema_type {
        Some("object") => validate_object_schema(schema),
        Some("array") => {
            // Check for unsupported array constraints
            if schema.get("minItems").is_some() || schema.get("maxItems").is_some() {
                return Err(DeepSeekError::SchemaError {
                    message: "Array minItems/maxItems not supported in strict mode".to_string(),
                    details: Some(schema.clone()),
                });
            }

            // Validate items schema if present
            if let Some(items) = schema.get("items") {
                validate_nested_schema(items)?;
            }
            Ok(())
        }
        Some("string") => {
            // Check for unsupported string constraints
            if schema.get("minLength").is_some() || schema.get("maxLength").is_some() {
                return Err(DeepSeekError::SchemaError {
                    message: "String minLength/maxLength not supported in strict mode".to_string(),
                    details: Some(schema.clone()),
                });
            }

            // Validate format if present
            if let Some(format) = schema.get("format").and_then(|f| f.as_str()) {
                const SUPPORTED_FORMATS: &[&str] = &["email", "hostname", "ipv4", "ipv6", "uuid"];
                if !SUPPORTED_FORMATS.contains(&format) {
                    return Err(DeepSeekError::SchemaError {
                        message: format!(
                            "String format '{format}' not supported in strict mode. Supported: {SUPPORTED_FORMATS:?}"
                        ),
                        details: Some(schema.clone()),
                    });
                }
            }
            Ok(())
        }
        Some("anyOf") => {
            // Validate each option in anyOf
            if let Some(options) = schema.get("anyOf").and_then(|a| a.as_array()) {
                for (i, option) in options.iter().enumerate() {
                    validate_nested_schema(option).map_err(|e| {
                        if let DeepSeekError::SchemaError { message, .. } = e {
                            DeepSeekError::SchemaError {
                                message: format!("In anyOf[{i}]: {message}"),
                                details: Some(schema.clone()),
                            }
                        } else {
                            e
                        }
                    })?;
                }
            }
            Ok(())
        }
        Some("number" | "integer" | "boolean" | "null") => Ok(()),
        Some(unknown) => Err(DeepSeekError::SchemaError {
            message: format!("Type '{unknown}' may not be supported in strict mode"),
            details: Some(schema.clone()),
        }),
        None => {
            // Check for refs or enums - both are supported
            // If no type, $ref, or enum, assume it's valid
            Ok(())
        }
    }
}

/// Validates tool arguments against a JSON schema
pub fn validate_tool_arguments(arguments: &str, schema: &Value, tool_name: &str) -> Result<Value> {
    // Parse arguments as JSON
    let args_value: Value =
        serde_json::from_str(arguments).map_err(|e| DeepSeekError::ToolArgumentsInvalid {
            message: format!("Failed to parse JSON: {e}"),
            tool_name: tool_name.to_string(),
            arguments: arguments.to_string(),
        })?;

    // Validate against schema
    let compiled = jsonschema::options()
        .with_draft(Draft::Draft7)
        .build(schema)
        .map_err(|e| DeepSeekError::SchemaError {
            message: format!("Invalid schema: {e}"),
            details: Some(schema.clone()),
        })?;

    if !compiled.is_valid(&args_value) {
        // Get validation errors
        let errors = compiled.iter_errors(&args_value);
        let error_messages: Vec<String> = errors
            .take(5) // Limit to first 5 errors
            .map(|e| format!("- {e}"))
            .collect();

        return Err(DeepSeekError::ToolArgumentsInvalid {
            message: format!(
                "Arguments do not match schema:\n{}",
                error_messages.join("\n")
            ),
            tool_name: tool_name.to_string(),
            arguments: arguments.to_string(),
        });
    }

    Ok(args_value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_valid_strict_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"}
            },
            "required": ["name", "age"],
            "additionalProperties": false
        });

        assert!(validate_strict_schema(&schema).is_ok());
    }

    #[test]
    fn test_missing_required_array() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            },
            "additionalProperties": false
        });

        let result = validate_strict_schema(&schema);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DeepSeekError::SchemaError { .. }
        ));
    }

    #[test]
    fn test_missing_additional_properties() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            },
            "required": ["name"]
        });

        let result = validate_strict_schema(&schema);
        assert!(result.is_err());
    }

    #[test]
    fn test_unsupported_string_constraint() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "minLength": 3
                }
            },
            "required": ["name"],
            "additionalProperties": false
        });

        let result = validate_strict_schema(&schema);
        assert!(result.is_err());
    }

    #[test]
    fn test_supported_string_format() {
        let schema = json!({
            "type": "object",
            "properties": {
                "email": {
                    "type": "string",
                    "format": "email"
                }
            },
            "required": ["email"],
            "additionalProperties": false
        });

        assert!(validate_strict_schema(&schema).is_ok());
    }

    #[test]
    fn test_validate_tool_arguments() {
        let schema = json!({
            "type": "object",
            "properties": {
                "message": {"type": "string"}
            },
            "required": ["message"],
            "additionalProperties": false
        });

        let valid_args = r#"{"message": "Hello, world!"}"#;
        let result = validate_tool_arguments(valid_args, &schema, "test_tool");
        assert!(result.is_ok());

        let invalid_args = r#"{"msg": "Hello"}"#;
        let result = validate_tool_arguments(invalid_args, &schema, "test_tool");
        assert!(result.is_err());
    }
}
