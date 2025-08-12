use serde_json::json;

/// MCP Protocol JSON Schema definitions based on spec
pub fn get_mcp_schema() -> serde_json::Value {
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "definitions": {
            "InitializeRequest": {
                "type": "object",
                "properties": {
                    "jsonrpc": {"const": "2.0"},
                    "id": {"type": ["string", "number", "null"]},
                    "method": {"const": "initialize"},
                    "params": {
                        "type": "object",
                        "properties": {
                            "protocolVersion": {"type": "string"},
                            "capabilities": {"type": "object"},
                            "clientInfo": {
                                "type": "object",
                                "properties": {
                                    "name": {"type": "string"},
                                    "version": {"type": "string"}
                                },
                                "required": ["name", "version"]
                            }
                        },
                        "required": ["protocolVersion", "capabilities", "clientInfo"]
                    }
                },
                "required": ["jsonrpc", "method", "params"]
            },
            "InitializeResponse": {
                "type": "object",
                "properties": {
                    "jsonrpc": {"const": "2.0"},
                    "id": {"type": ["string", "number", "null"]},
                    "result": {
                        "type": "object",
                        "properties": {
                            "protocolVersion": {"type": "string"},
                            "capabilities": {"type": "object"},
                            "serverInfo": {
                                "type": "object",
                                "properties": {
                                    "name": {"type": "string"},
                                    "version": {"type": "string"}
                                },
                                "required": ["name", "version"]
                            },
                            "instructions": {"type": "string"}
                        },
                        "required": ["protocolVersion", "capabilities", "serverInfo"]
                    }
                },
                "required": ["jsonrpc", "id", "result"]
            },
            "ToolsListRequest": {
                "type": "object",
                "properties": {
                    "jsonrpc": {"const": "2.0"},
                    "id": {"type": ["string", "number", "null"]},
                    "method": {"const": "tools/list"},
                    "params": {
                        "type": "object",
                        "properties": {
                            "cursor": {"type": "string"}
                        }
                    }
                },
                "required": ["jsonrpc", "method"]
            },
            "ToolsListResponse": {
                "type": "object",
                "properties": {
                    "jsonrpc": {"const": "2.0"},
                    "id": {"type": ["string", "number", "null"]},
                    "result": {
                        "type": "object",
                        "properties": {
                            "tools": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "name": {"type": "string"},
                                        "description": {"type": "string"},
                                        "inputSchema": {
                                            "type": "object",
                                            "properties": {
                                                "type": {"const": "object"},
                                                "properties": {"type": "object"},
                                                "required": {"type": "array", "items": {"type": "string"}}
                                            },
                                            "required": ["type"]
                                        }
                                    },
                                    "required": ["name", "inputSchema"]
                                }
                            },
                            "nextCursor": {"type": "string"}
                        },
                        "required": ["tools"]
                    }
                },
                "required": ["jsonrpc", "id", "result"]
            },
            "ToolCallRequest": {
                "type": "object",
                "properties": {
                    "jsonrpc": {"const": "2.0"},
                    "id": {"type": ["string", "number", "null"]},
                    "method": {"const": "tools/call"},
                    "params": {
                        "type": "object",
                        "properties": {
                            "name": {"type": "string"},
                            "arguments": {"type": "object"}
                        },
                        "required": ["name", "arguments"]
                    }
                },
                "required": ["jsonrpc", "method", "params"]
            },
            "ToolCallResponse": {
                "type": "object",
                "properties": {
                    "jsonrpc": {"const": "2.0"},
                    "id": {"type": ["string", "number", "null"]},
                    "result": {
                        "type": "object",
                        "properties": {
                            "content": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "type": {"type": "string", "enum": ["text", "image", "resource"]},
                                        "text": {"type": "string"},
                                        "data": {"type": "string"},
                                        "mimeType": {"type": "string"},
                                        "uri": {"type": "string"},
                                        "resource": {"type": "object"}
                                    }
                                }
                            },
                            "isError": {"type": "boolean"},
                            "_meta": {"type": "object"}
                        },
                        "required": ["content"]
                    }
                },
                "required": ["jsonrpc", "id", "result"]
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonschema;
    use serde_json::json;

    #[test]
    fn test_initialize_response_schema() {
        let schema = get_mcp_schema();
        let compiled =
            jsonschema::validator_for(&schema["definitions"]["InitializeResponse"]).unwrap();

        // Valid response
        let valid = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {
                    "name": "arkavo",
                    "version": "0.2.0"
                }
            }
        });
        assert!(compiled.is_valid(&valid));

        // Invalid - missing protocolVersion
        let invalid = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "capabilities": {},
                "serverInfo": {
                    "name": "arkavo",
                    "version": "0.2.0"
                }
            }
        });
        assert!(!compiled.is_valid(&invalid));
    }

    #[test]
    fn test_tools_list_response_schema() {
        let schema = get_mcp_schema();
        let compiled =
            jsonschema::validator_for(&schema["definitions"]["ToolsListResponse"]).unwrap();

        // Valid response
        let valid = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "tools": [
                    {
                        "name": "test_tool",
                        "description": "A test tool",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "param": {"type": "string"}
                            }
                        }
                    }
                ]
            }
        });
        assert!(compiled.is_valid(&valid));

        // Invalid - missing inputSchema
        let invalid = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "tools": [
                    {
                        "name": "test_tool",
                        "description": "A test tool"
                    }
                ]
            }
        });
        assert!(!compiled.is_valid(&invalid));
    }
}
