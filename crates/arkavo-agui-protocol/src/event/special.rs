use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Pass-through container for external events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawFields {
    pub event: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Application-specific extension event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomFields {
    pub name: String,
    pub value: Value,
}
