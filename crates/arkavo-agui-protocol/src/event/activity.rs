use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::patch::JsonPatch;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySnapshotFields {
    pub message_id: String,
    pub activity_type: String,
    pub content: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replace: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDeltaFields {
    pub message_id: String,
    pub activity_type: String,
    pub patch: Vec<JsonPatch>,
}
