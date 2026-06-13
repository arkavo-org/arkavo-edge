use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::message::Message;
use crate::patch::JsonPatch;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSnapshotFields {
    pub snapshot: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateDeltaFields {
    pub delta: Vec<JsonPatch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesSnapshotFields {
    pub messages: Vec<Message>,
}
