//! The marker that says a message is a step of a pipeline run.
//!
//! The marker is for the receiving agent. A message that carries it is
//! answered on its own, with no conversation behind it, and never starts a
//! pipeline of its own.
//!
//! It is not a credential. The RPC endpoint does not authenticate callers,
//! so any caller that can reach the agent can set it, and what it buys is
//! limited to what is described here: an answer from this agent alone.

use std::time::Duration;

use serde_json::Value;

/// Metadata key of the pipeline marker.
pub const MARKER_KEY: &str = "pipeline";

/// True when `metadata` marks its message as a step of a pipeline run.
///
/// The key alone decides. A marker that is malformed still means the sender
/// intended a step, and treating it as an ordinary message would start a
/// second pipeline inside the first.
pub fn is_marked(metadata: Option<&Value>) -> bool {
    metadata
        .and_then(|m| m.get(MARKER_KEY))
        .is_some_and(|marker| !marker.is_null())
}

/// How long the sender of a marked message will wait for the answer, when
/// the marker says.
pub fn step_timeout(metadata: Option<&Value>) -> Option<Duration> {
    metadata?
        .get(MARKER_KEY)?
        .get("timeout_ms")?
        .as_u64()
        .map(Duration::from_millis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_marker_key_marks_a_message() {
        assert!(is_marked(Some(
            &json!({"pipeline": {"run_id": "r", "step": 2}})
        )));
        assert!(is_marked(Some(&json!({"pipeline": {}}))));
        assert!(is_marked(Some(&json!({"pipeline": "yes"}))));
        assert!(is_marked(Some(&json!({"pipeline": false}))));
    }

    #[test]
    fn a_message_without_the_key_is_not_marked() {
        assert!(!is_marked(None));
        assert!(!is_marked(Some(&json!({}))));
        assert!(!is_marked(Some(&json!({"source": "pipeline"}))));
        assert!(!is_marked(Some(&json!({"pipeline": null}))));
        assert!(!is_marked(Some(&json!({"nested": {"pipeline": {}}}))));
        assert!(!is_marked(Some(&json!("pipeline"))));
    }

    #[test]
    fn the_senders_wait_is_read_from_the_marker() {
        assert_eq!(
            step_timeout(Some(&json!({"pipeline": {"timeout_ms": 90_000}}))),
            Some(Duration::from_secs(90))
        );
    }

    #[test]
    fn a_marker_without_a_usable_wait_sets_none() {
        for metadata in [
            json!({"pipeline": {}}),
            json!({"pipeline": {"timeout_ms": "soon"}}),
            json!({"pipeline": {"timeout_ms": -5}}),
            json!({"pipeline": true}),
            json!({"timeout_ms": 90_000}),
        ] {
            assert_eq!(step_timeout(Some(&metadata)), None, "{metadata}");
        }
        assert_eq!(step_timeout(None), None);
    }
}
