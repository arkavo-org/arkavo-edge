use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Permit,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    pub attribute_value_fqns: Vec<String>,
}

impl Resource {
    pub fn new(fqns: Vec<String>) -> Self {
        Self {
            attribute_value_fqns: fqns,
        }
    }

    pub fn mcp_tool(tool_name: &str) -> Self {
        let slug = crate::cwt_subject::tool_value_slug(tool_name);
        Self {
            attribute_value_fqns: vec![format!("https://arkavo.net/attr/mcp-tool/value/{slug}")],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub name: String,
}

impl Action {
    pub fn tools_call() -> Self {
        Self {
            name: "tools/call".to_string(),
        }
    }

    pub fn tools_list() -> Self {
        Self {
            name: "tools/list".to_string(),
        }
    }

    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthzenEvaluationResponse {
    pub decision: bool,
    #[serde(default)]
    pub context: Option<serde_json::Value>,
}

pub struct McpToolMapping;

impl McpToolMapping {
    /// OpenTDF attribute values cannot contain `.`; dots become underscores.
    pub fn tool_to_resource(tool_name: &str) -> Resource {
        Resource::mcp_tool(tool_name)
    }

    /// `list_tools` is a normal `tools/call` tool, not a diagnostic bypass.
    /// `status` / `health` / `version` remain a documented 90-day exception.
    pub fn is_safe_diagnostic(tool_name: &str) -> bool {
        matches!(tool_name, "status" | "health" | "version")
    }
}
