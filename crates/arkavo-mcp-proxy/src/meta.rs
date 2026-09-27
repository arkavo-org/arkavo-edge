//! The `_meta.arkavo` credentials a `tools/call` carries, and their removal
//! before the call is forwarded.
//!
//! A client presents its permit and its proof of possession as two base64url
//! strings under `params._meta.arkavo`. Reading them is the proxy's first
//! contact with untrusted input, so each is bounded by what that field can
//! actually hold before anything decodes it, and a field that cannot be used
//! keeps saying *why* rather than collapsing into "absent".
//!
//! The same key is removed on the way out: an allowed call is forwarded
//! without it, so the live credentials never reach the upstream server.

// `pub(crate)` is the real, intended visibility here (the module is private,
// so nothing leaks past the crate either way); `redundant_pub_crate` wants
// `pub`, which `unreachable_pub` then rejects.
#![allow(clippy::redundant_pub_crate)]

use crate::policy::Credential;
use serde_json::Value;

/// The longest base64url string `_meta.arkavo.permit` may carry: the permit
/// size cap re-expressed in encoded characters (four per three bytes, plus a
/// partial group). Anything longer cannot decode to a permit this stack would
/// accept, so it is refused without decoding it.
const MAX_ENCODED_PERMIT: usize = 4 * arkavo_dispatch_gate::MAX_PERMIT_BYTES / 3 + 4;

/// The longest base64url string `_meta.arkavo.pop` may carry: the length of
/// one signature re-expressed in encoded characters, by the same formula.
///
/// A proof of possession is exactly one signature —
/// `arkavo_permit::SIGNATURE_BYTES` from both key types this stack signs
/// with, Ed25519 and P-256 in P1363 form — which is 86 characters unpadded.
/// Bounding it by the permit's cap instead would let a caller send 21 849
/// characters of base64 for something that can only ever be 86, and hold the
/// difference in memory while it was decoded.
const MAX_ENCODED_PROOF: usize = 4 * arkavo_dispatch_gate::SIGNATURE_BYTES / 3 + 4;

/// The permit and the proof of possession a `tools/call`'s params carry,
/// each bounded and decoded by its own rule.
pub(crate) fn credentials(params: Option<&Value>) -> (Credential, Credential) {
    let meta = params
        .and_then(|p| p.get("_meta"))
        .and_then(|m| m.get("arkavo"));
    (
        credential(meta, "permit", MAX_ENCODED_PERMIT),
        credential(meta, "pop", MAX_ENCODED_PROOF),
    )
}

/// Read one `_meta.arkavo` credential, keeping *why* it is unusable.
///
/// `max_encoded` is this field's own bound: a permit and a proof of
/// possession differ by three orders of magnitude in what they can
/// legitimately be, so one cap for both is barely a cap on the proof.
fn credential(meta: Option<&Value>, key: &str, max_encoded: usize) -> Credential {
    match meta.and_then(|m| m.get(key)) {
        None => Credential::Absent,
        Some(value) => match value.as_str() {
            // A non-string is as unusable as a malformed string, and saying
            // so is more use to the client than calling it absent.
            None => Credential::Undecodable,
            Some(text) if text.len() > max_encoded => Credential::Oversized,
            Some(text) => decode_b64url(text).map_or(Credential::Undecodable, Credential::Present),
        },
    }
}

/// Decode a base64url-without-padding string, as used by `_meta.arkavo`.
fn decode_b64url(text: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text)
        .ok()
}

/// Strip the `arkavo` key out of `params._meta` before forwarding an
/// allowed call upstream, so the live permit and proof-of-possession never
/// leave the proxy. Every other `_meta` key travels unchanged; `_meta`
/// itself is dropped only if stripping `arkavo` leaves it empty.
pub(crate) fn strip_arkavo_meta(params: Option<&Value>) -> Option<Value> {
    let mut params = params?.clone();
    if let Some(object) = params.as_object_mut() {
        let empty = object
            .get_mut("_meta")
            .and_then(Value::as_object_mut)
            .map(|meta| {
                meta.remove("arkavo");
                meta.is_empty()
            });
        if empty == Some(true) {
            object.remove("_meta");
        }
    }
    Some(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;
    use serde_json::json;

    #[test]
    fn decode_b64url_accepts_only_unpadded_base64url() {
        assert_eq!(decode_b64url("aGk").as_deref(), Some(&b"hi"[..]));
        assert_eq!(decode_b64url("").as_deref(), Some(&b""[..]));
        // Padding, the standard alphabet and stray characters are refused
        // rather than silently decoding to something else.
        assert_eq!(decode_b64url("aGk="), None);
        assert_eq!(decode_b64url("a+/b"), None);
        assert_eq!(decode_b64url("not base64!"), None);
    }

    /// The ways a credential can be unusable have to stay distinguishable: a
    /// client that sent nothing, one whose encoding is wrong, and one whose
    /// field is too long to be what it claims to be.
    #[test]
    #[spec("PDG-009")]
    fn credential_distinguishes_absent_undecodable_and_oversized() {
        let meta = json!({
            "permit": "aGk",
            "pop": "!!! not base64",
            "huge": "A".repeat(MAX_ENCODED_PERMIT + 1),
            "number": 7,
        });
        let meta = Some(&meta);

        assert_eq!(
            credential(meta, "permit", MAX_ENCODED_PERMIT),
            Credential::Present(b"hi".to_vec())
        );
        assert_eq!(
            credential(meta, "pop", MAX_ENCODED_PROOF),
            Credential::Undecodable
        );
        assert_eq!(
            credential(meta, "huge", MAX_ENCODED_PERMIT),
            Credential::Oversized
        );
        assert_eq!(
            credential(meta, "number", MAX_ENCODED_PERMIT),
            Credential::Undecodable
        );
        assert_eq!(
            credential(meta, "missing", MAX_ENCODED_PERMIT),
            Credential::Absent
        );
        assert_eq!(
            credential(None, "permit", MAX_ENCODED_PERMIT),
            Credential::Absent
        );
    }

    /// The permit and proof are the proxy's own credentials and must not
    /// travel upstream, but the rest of `_meta` is the client's and must.
    #[test]
    #[spec("PDG-008")]
    fn strip_arkavo_meta_removes_only_the_arkavo_key() {
        let params = json!({
            "name": "echo",
            "arguments": {"n": 1},
            "_meta": {"arkavo": {"permit": "p", "pop": "q"}, "trace": "t-1"},
        });
        let stripped = strip_arkavo_meta(Some(&params)).expect("params");
        assert_eq!(stripped["_meta"], json!({"trace": "t-1"}));
        assert_eq!(stripped["arguments"], json!({"n": 1}));

        // `_meta` itself goes only when nothing else was in it.
        let only_arkavo = json!({"name": "echo", "_meta": {"arkavo": {"permit": "p"}}});
        let stripped = strip_arkavo_meta(Some(&only_arkavo)).expect("params");
        assert!(stripped.get("_meta").is_none(), "stripped: {stripped}");

        // Params without `_meta` are forwarded untouched, and a call with no
        // params at all stays that way.
        let bare = json!({"name": "echo"});
        assert_eq!(strip_arkavo_meta(Some(&bare)), Some(bare));
        assert_eq!(strip_arkavo_meta(None), None);
    }

    /// A string at the cap is still decoded: the bound refuses what cannot be
    /// the credential it claims to be, not what merely approaches its size.
    #[test]
    fn a_credential_at_the_encoded_cap_is_not_oversized() {
        let meta = json!({
            "permit": "A".repeat(MAX_ENCODED_PERMIT),
            "pop": "A".repeat(MAX_ENCODED_PROOF),
        });
        let meta = Some(&meta);
        assert_ne!(
            credential(meta, "permit", MAX_ENCODED_PERMIT),
            Credential::Oversized
        );
        assert_ne!(
            credential(meta, "pop", MAX_ENCODED_PROOF),
            Credential::Oversized
        );
    }

    /// The proof's own bound is what makes it a bound at all: a 64-byte
    /// signature is 86 characters, and everything from there to the permit's
    /// 21 849 was accepted while the two shared one cap.
    #[test]
    #[spec("PDG-009")]
    fn a_proof_longer_than_a_signature_is_oversized() {
        let real_proof = "A".repeat(86);
        assert!(real_proof.len() <= MAX_ENCODED_PROOF);

        let meta = json!({
            "pop": "A".repeat(MAX_ENCODED_PROOF + 1),
            "permit": "A".repeat(MAX_ENCODED_PROOF + 1),
        });
        let meta = Some(&meta);
        assert_eq!(
            credential(meta, "pop", MAX_ENCODED_PROOF),
            Credential::Oversized
        );
        // The same string is nowhere near the permit's cap, which is the
        // point: one cap for both fields left the proof unbounded in practice.
        assert_ne!(
            credential(meta, "permit", MAX_ENCODED_PERMIT),
            Credential::Oversized
        );
    }
}
