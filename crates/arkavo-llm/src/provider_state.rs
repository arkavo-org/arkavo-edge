//! Opaque provider continuation state, tagged with the wire family that made it.
//!
//! Providers such as OpenAI Responses hand back ordered output items — encrypted
//! reasoning, native call records — that only that same wire family can consume.
//! Carrying them as a bare `Vec<Value>` on the provider-neutral message types let
//! any adapter forward another provider's private state, and forced every
//! consumer to re-derive meaning from string keys. The tag makes provenance part
//! of the value, so replay is a typed question rather than a convention.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The provider wire family that produced a batch of opaque items.
///
/// The wire names are written into persisted sessions, so they are spelled out
/// per variant rather than derived from the Rust identifier.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderStateTag {
    /// No provider claimed this state; the item list is empty.
    #[default]
    #[serde(rename = "empty")]
    Empty,
    /// OpenAI Responses `/v1/responses` `output` items.
    #[serde(rename = "openai_responses")]
    OpenAiResponses,
}

/// Provider-owned conversation items plus the wire family that produced them.
///
/// The items are opaque: never rendered, never inspected outside the accessors
/// here, and replayable only to the family named by the tag.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ProviderState {
    #[serde(default)]
    tag: ProviderStateTag,
    #[serde(default)]
    items: Vec<Value>,
}

impl ProviderState {
    /// Capture OpenAI Responses output items.
    ///
    /// An empty batch carries no provenance, so it collapses to the untagged
    /// default; that keeps the invariant that a tag is set exactly when items
    /// are present, and lets `is_empty` alone decide "nothing to replay".
    pub fn openai_responses(items: Vec<Value>) -> Self {
        if items.is_empty() {
            return Self::default();
        }
        Self {
            tag: ProviderStateTag::OpenAiResponses,
            items,
        }
    }

    /// Whether there is anything replayable here.
    ///
    /// Untagged items count as nothing: a hand-edited or truncated blob can
    /// deserialize to items with no tag, and such items name no wire format, so
    /// no provider could ever accept them. Reading the tag here — not just the
    /// item count — is what stops `is_empty` and `replay_items_for` from
    /// disagreeing, and lets `skip_serializing_if` drop the orphaned items on
    /// the next save instead of carrying them forever.
    pub fn is_empty(&self) -> bool {
        self.tag == ProviderStateTag::Empty || self.items.is_empty()
    }

    /// The items to replay to `tag`, or `None` when another family produced them
    /// or there is nothing to replay.
    ///
    /// The only way to read the items back out, so a caller cannot reach them
    /// without naming the wire format it is about to write them into — which is
    /// what makes cross-provider replay impossible rather than discouraged.
    pub fn replay_items_for(self, tag: ProviderStateTag) -> Option<Vec<Value>> {
        (self.tag == tag && !self.is_empty()).then_some(self.items)
    }

    /// Whether the provider itself recorded tool calls this turn.
    ///
    /// Distinguishes provider-native calls from tool syntax a local parser
    /// extracted from prose, which the provider has no record of.
    pub fn has_native_calls(&self) -> bool {
        self.native_items()
            .any(|item| item["type"] == "function_call")
    }

    /// Native calls as `(call_id, tool_name)`, in the order the provider emitted
    /// them. A call the provider never gave an id to cannot be answered, so it is
    /// not reported here; the name is empty when the provider omitted it.
    pub fn native_calls(&self) -> impl Iterator<Item = (&str, &str)> {
        self.native_items().filter_map(|item| {
            if item["type"] != "function_call" {
                return None;
            }
            Some((
                item["call_id"].as_str()?,
                item["name"].as_str().unwrap_or_default(),
            ))
        })
    }

    /// Ids of the native calls the next request must answer.
    pub fn native_call_ids(&self) -> impl Iterator<Item = &str> {
        self.native_calls().map(|(id, _)| id)
    }

    /// Items whose shape this crate knows how to interpret. A future family
    /// with different item keys must not be read with Responses' vocabulary.
    fn native_items(&self) -> std::slice::Iter<'_, Value> {
        match self.tag {
            ProviderStateTag::OpenAiResponses => self.items.iter(),
            ProviderStateTag::Empty => [].iter(),
        }
    }
}

/// Opaque state must never reach a log line; only its provenance and size do.
impl std::fmt::Debug for ProviderState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderState")
            .field("tag", &self.tag)
            .field("item_count", &self.items.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;
    use serde_json::json;

    fn responses_turn() -> ProviderState {
        ProviderState::openai_responses(vec![
            json!({"type":"reasoning","id":"rs_1","encrypted_content":"opaque-canary","summary":[]}),
            json!({"type":"function_call","call_id":"fc_1","name":"read","arguments":"{}"}),
            json!({"type":"function_call","call_id":"fc_2","name":"write","arguments":"{}"}),
        ])
    }

    #[spec("ASTRA-002")]
    #[test]
    fn state_replays_only_to_the_family_that_produced_it() {
        let state = responses_turn();
        assert_eq!(
            state
                .clone()
                .replay_items_for(ProviderStateTag::OpenAiResponses)
                .map(|items| items.len()),
            Some(3)
        );
        assert!(
            state.replay_items_for(ProviderStateTag::Empty).is_none(),
            "another wire family must not receive OpenAI items"
        );
    }

    #[spec("ASTRA-002")]
    #[test]
    fn empty_state_is_replayable_by_nobody() {
        let state = ProviderState::default();
        assert!(state.is_empty());
        assert!(!state.has_native_calls());
        assert_eq!(state.native_call_ids().count(), 0);
        assert!(
            state
                .clone()
                .replay_items_for(ProviderStateTag::OpenAiResponses)
                .is_none()
        );
        // No items means no provenance to record, so the tag stays unset.
        assert_eq!(ProviderState::openai_responses(Vec::new()), state);
    }

    /// A blob that lost its tag — hand-edited, truncated, or written by an older
    /// build — deserializes to items no wire format claims. It must read as
    /// empty everywhere, or `is_empty` would say "state present" while every
    /// replay path refuses it, and the orphaned items would be re-saved forever.
    #[spec("ASTRA-002")]
    #[test]
    fn untagged_items_read_as_empty_and_are_dropped_on_the_next_save() {
        let orphaned: ProviderState = serde_json::from_str(
            r#"{"items":[{"type":"function_call","call_id":"fc_1","name":"read","arguments":"{}"}]}"#,
        )
        .unwrap();
        assert!(orphaned.is_empty());
        assert!(!orphaned.has_native_calls());
        assert_eq!(orphaned.native_call_ids().count(), 0);
        assert!(
            orphaned
                .clone()
                .replay_items_for(ProviderStateTag::OpenAiResponses)
                .is_none()
        );
        // `Message`/`StreamResponse` skip serializing an empty state, so the
        // next save drops the orphans rather than carrying them another round.
        let message = crate::Message {
            provider_state: orphaned,
            ..crate::Message::assistant("visible answer")
        };
        let json = serde_json::to_string(&message).unwrap();
        assert!(!json.contains("provider_state"));
        assert!(!json.contains("fc_1"));
    }

    #[spec("ASTRA-002")]
    #[test]
    fn native_calls_come_from_the_captured_responses_output() {
        let state = responses_turn();
        assert!(state.has_native_calls());
        assert_eq!(
            state.native_call_ids().collect::<Vec<_>>(),
            vec!["fc_1", "fc_2"]
        );
        assert_eq!(
            state.native_calls().collect::<Vec<_>>(),
            vec![("fc_1", "read"), ("fc_2", "write")]
        );
    }

    #[spec("ASTRA-002")]
    #[test]
    fn a_turn_without_native_calls_reports_none() {
        let state = ProviderState::openai_responses(vec![
            json!({"type":"reasoning","id":"rs_1","summary":[]}),
            json!({"type":"message","role":"assistant","content":[]}),
        ]);
        assert!(!state.is_empty());
        assert!(!state.has_native_calls());
        assert_eq!(state.native_call_ids().count(), 0);
    }

    #[spec("ASTRA-002")]
    #[test]
    fn debug_reports_provenance_and_size_but_never_the_items() {
        let rendered = format!("{:?}", responses_turn());
        assert!(!rendered.contains("opaque-canary"));
        assert!(rendered.contains("OpenAiResponses"));
        assert!(rendered.contains("item_count: 3"));
    }

    #[spec("ASTRA-002")]
    #[test]
    fn state_survives_a_persistence_round_trip_with_its_tag() {
        let state = responses_turn();
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("openai_responses"));
        let restored: ProviderState = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, state);
        assert_eq!(
            restored
                .replay_items_for(ProviderStateTag::OpenAiResponses)
                .map(|items| items.len()),
            Some(3)
        );
    }
}
