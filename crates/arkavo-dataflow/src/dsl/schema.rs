use serde_json::Value;

pub const BLUEPRINT_SCHEMA: &str = r##"{
    "$schema": "http://json-schema.org/draft-07/schema#",
    "title": "Arkavo Dataflow Blueprint",
    "type": "object",
    "required": ["version", "nodes", "links"],
    "properties": {
        "version": {
            "type": "string",
            "pattern": "^\\d+\\.\\d+\\.\\d+$",
            "description": "Semantic version of the blueprint format"
        },
        "name": {
            "type": "string",
            "description": "Optional name for the pipeline"
        },
        "description": {
            "type": "string",
            "description": "Optional description of the pipeline's purpose"
        },
        "nodes": {
            "type": "array",
            "items": { "$ref": "#/definitions/node" },
            "description": "List of processing nodes in the pipeline"
        },
        "links": {
            "type": "array",
            "items": { "$ref": "#/definitions/link" },
            "description": "List of connections between nodes"
        },
        "metadata": {
            "type": "object",
            "description": "Optional metadata for the pipeline"
        }
    },
    "definitions": {
        "node": {
            "type": "object",
            "required": ["id", "kind"],
            "properties": {
                "id": {
                    "type": "string",
                    "pattern": "^[a-zA-Z0-9_-]+$",
                    "description": "Unique identifier for the node"
                },
                "kind": {
                    "enum": ["source", "transform", "sink", "router"],
                    "description": "Type of node"
                },
                "params": {
                    "type": "object",
                    "description": "Node-specific parameters"
                },
                "auth_ref": {
                    "type": "string",
                    "pattern": "^\\$\\{[A-Z_]+\\}$",
                    "description": "Optional environment variable reference for authentication"
                }
            }
        },
        "link": {
            "type": "object",
            "required": ["from", "to"],
            "properties": {
                "from": {
                    "type": "string",
                    "description": "Source node ID"
                },
                "to": {
                    "type": "string",
                    "description": "Target node ID"
                },
                "rule": {
                    "$ref": "#/definitions/rule",
                    "description": "Optional rule for filtering or transforming messages"
                }
            }
        },
        "rule": {
            "type": "object",
            "required": ["type"],
            "properties": {
                "type": {
                    "enum": ["filter", "transform", "split", "merge", "passthrough"],
                    "description": "Type of rule"
                },
                "conditions": {
                    "type": "array",
                    "items": { "$ref": "#/definitions/condition" },
                    "description": "Conditions for filtering (used with filter type)"
                },
                "transform": {
                    "$ref": "#/definitions/transform",
                    "description": "Transform configuration (used with transform type)"
                },
                "split_config": {
                    "$ref": "#/definitions/split_config",
                    "description": "Split configuration (used with split type)"
                },
                "merge_config": {
                    "$ref": "#/definitions/merge_config",
                    "description": "Merge configuration (used with merge type)"
                }
            }
        },
        "condition": {
            "type": "object",
            "required": ["field", "operator", "value"],
            "properties": {
                "field": {
                    "type": "string",
                    "description": "Field path to test (e.g., 'message', 'payload.level')"
                },
                "operator": {
                    "enum": ["contains", "equals", "matches", "gt", "lt", "gte", "lte", "exists", "not_exists", "starts_with", "ends_with"],
                    "description": "Comparison operator"
                },
                "value": {
                    "description": "Value to compare against"
                }
            }
        },
        "transform": {
            "type": "object",
            "required": ["type", "params"],
            "properties": {
                "type": {
                    "enum": ["extract", "format", "aggregate", "enrich", "map", "reduce"],
                    "description": "Type of transformation"
                },
                "params": {
                    "type": "object",
                    "description": "Transform-specific parameters"
                }
            }
        },
        "split_config": {
            "type": "object",
            "required": ["strategy", "targets"],
            "properties": {
                "strategy": {
                    "enum": ["round_robin", "hash", "broadcast", "conditional"],
                    "description": "How to distribute messages"
                },
                "targets": {
                    "type": "array",
                    "items": { "type": "string" },
                    "minItems": 2,
                    "description": "Target node IDs"
                }
            }
        },
        "merge_config": {
            "type": "object",
            "required": ["strategy"],
            "properties": {
                "strategy": {
                    "enum": ["first", "all", "concat", "vote"],
                    "description": "How to combine messages"
                },
                "timeout_ms": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Optional timeout for waiting on multiple inputs"
                }
            }
        }
    }
}"##;

pub fn validate_json_schema(json: &Value) -> Result<(), Vec<String>> {
    let schema =
        serde_json::from_str(BLUEPRINT_SCHEMA).expect("Blueprint schema should be valid JSON");

    let compiled =
        jsonschema::JSONSchema::compile(&schema).expect("Blueprint schema should compile");

    let is_valid = compiled.is_valid(json);

    if is_valid {
        Ok(())
    } else {
        // For jsonschema 0.18, we'll just return a simple error
        Err(vec!["Blueprint validation failed".to_string()])
    }
}

pub fn get_schema_json() -> Value {
    serde_json::from_str(BLUEPRINT_SCHEMA).expect("Blueprint schema should be valid JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_valid_blueprint_schema() {
        let blueprint = json!({
            "version": "1.0.0",
            "name": "Test Pipeline",
            "nodes": [
                {
                    "id": "input",
                    "kind": "source",
                    "params": {}
                },
                {
                    "id": "filter",
                    "kind": "transform",
                    "params": {
                        "model": "gpt-4"
                    }
                }
            ],
            "links": [
                {
                    "from": "input",
                    "to": "filter",
                    "rule": {
                        "type": "filter",
                        "conditions": [
                            {
                                "field": "message",
                                "operator": "contains",
                                "value": "error"
                            }
                        ]
                    }
                }
            ]
        });

        assert!(validate_json_schema(&blueprint).is_ok());
    }

    #[test]
    fn test_invalid_version_format() {
        let blueprint = json!({
            "version": "1.0",  // Missing patch version
            "nodes": [],
            "links": []
        });

        assert!(validate_json_schema(&blueprint).is_err());
    }

    #[test]
    fn test_auth_ref_validation() {
        let blueprint = json!({
            "version": "1.0.0",
            "nodes": [
                {
                    "id": "api",
                    "kind": "source",
                    "params": {},
                    "auth_ref": "${API_KEY}"  // Valid format
                }
            ],
            "links": []
        });

        assert!(validate_json_schema(&blueprint).is_ok());

        let invalid_blueprint = json!({
            "version": "1.0.0",
            "nodes": [
                {
                    "id": "api",
                    "kind": "source",
                    "params": {},
                    "auth_ref": "plain_text_key"  // Invalid - not env var reference
                }
            ],
            "links": []
        });

        assert!(validate_json_schema(&invalid_blueprint).is_err());
    }
}
