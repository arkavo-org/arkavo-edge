//! The signing profile envelopes and revocations share (spec §7.2): Ed25519
//! over the BLAKE3 digest of the JCS form, the profile §8.1.2 uses for
//! skills.

// Shared by the crate's modules only; plain `pub` here would trip
// `unreachable_pub`.
#![allow(clippy::redundant_pub_crate)]

use arkavo_crypto::{AgentKeypair, AgentPublicKey};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Map, Value, json};

use crate::protocol::{Refusal, RefusalReason};

/// The only signature algorithm draft-02 defines.
pub const ALGORITHM: &str = "Ed25519";

/// The JCS form's BLAKE3 digest, base64url without padding: the
/// `envelope_blake3` of a ready acknowledgement (§7.3.2).
pub fn digest_b64(value: &Value) -> String {
    URL_SAFE_NO_PAD.encode(digest(value))
}

fn digest(value: &Value) -> [u8; 32] {
    *blake3::hash(arkavo_swarmkit::canonical_json(value).as_bytes()).as_bytes()
}

/// Wrap `body` as `{member: body}` and sign it, adding `body.signing`.
pub(crate) fn sign(member: &str, body: Value, keypair: &AgentKeypair) -> Value {
    let mut outer = Value::Object(Map::from_iter([(member.to_string(), body)]));
    let signature = keypair.sign(&digest(&outer));
    outer[member]["signing"] = json!({
        "algorithm": ALGORITHM,
        "delegation_signature": URL_SAFE_NO_PAD.encode(signature),
    });
    outer
}

/// Split `{member: {..., signing}}` into its body without `signing`, after
/// checking the signature against the body's `orchestrator` key (§7.3 steps
/// 1 and 2).
pub(crate) fn verified_body(member: &str, outer: &Value) -> Result<Value, Refusal> {
    let invalid = |detail: String| Refusal::new(RefusalReason::InvalidEnvelope, detail);
    let Some(Value::Object(fields)) = outer.get(member) else {
        return Err(invalid(format!("expected an object named {member}")));
    };
    if outer.as_object().is_some_and(|o| o.len() != 1) {
        return Err(invalid(format!(
            "only {member} is allowed at the top level"
        )));
    }
    let mut body = fields.clone();
    let signing = body
        .remove("signing")
        .ok_or_else(|| invalid("signing is missing".into()))?;
    let algorithm = signing.get("algorithm").and_then(Value::as_str);
    if algorithm != Some(ALGORITHM) {
        return Err(invalid(format!(
            "signing.algorithm must be {ALGORITHM}, not {}",
            algorithm.unwrap_or("absent")
        )));
    }
    let signature = signing
        .get("delegation_signature")
        .and_then(Value::as_str)
        .and_then(|s| URL_SAFE_NO_PAD.decode(s).ok())
        .ok_or_else(|| invalid("signing.delegation_signature is not base64url".into()))?;
    if signing.as_object().is_some_and(|s| s.len() != 2) {
        return Err(invalid(
            "signing has fields other than algorithm and delegation_signature".into(),
        ));
    }
    let orchestrator = body
        .get("orchestrator")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("orchestrator is missing".into()))?;
    let key = AgentPublicKey::from_did_key(orchestrator)
        .map_err(|e| invalid(format!("orchestrator is not an Ed25519 did:key: {e}")))?;
    let unsigned = Value::Object(Map::from_iter([(
        member.to_string(),
        Value::Object(body.clone()),
    )]));
    key.verify(&digest(&unsigned), &signature).map_err(|_| {
        Refusal::new(
            RefusalReason::BadSignature,
            "delegation_signature does not verify",
        )
    })?;
    Ok(Value::Object(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_covers_every_field_and_only_its_key() {
        let keypair = AgentKeypair::generate();
        let did = keypair.public_key().to_did_key();
        let signed = sign(
            "delegation",
            json!({"orchestrator": did, "role_id": "artist", "n": 0.2}),
            &keypair,
        );
        let body = verified_body("delegation", &signed).unwrap();
        assert_eq!(body["role_id"], "artist");
        assert!(body.get("signing").is_none());

        let mut altered = signed.clone();
        altered["delegation"]["role_id"] = json!("writer");
        let refused = verified_body("delegation", &altered).unwrap_err();
        assert_eq!(refused.reason, RefusalReason::BadSignature);

        let other = AgentKeypair::generate().public_key().to_did_key();
        let mut claimed = signed;
        claimed["delegation"]["orchestrator"] = json!(other);
        let refused = verified_body("delegation", &claimed).unwrap_err();
        assert_eq!(refused.reason, RefusalReason::BadSignature);
    }

    #[test]
    fn the_wire_form_verifies_after_a_round_trip_through_text() {
        let keypair = AgentKeypair::generate();
        let did = keypair.public_key().to_did_key();
        let signed = sign(
            "delegation",
            json!({"orchestrator": did, "temperature": 0.2_f32, "z": "é\n", "a": [1, 2.5]}),
            &keypair,
        );
        let received: Value =
            serde_json::from_str(&serde_json::to_string(&signed).unwrap()).unwrap();
        assert!(verified_body("delegation", &received).is_ok());
        assert_eq!(digest_b64(&signed), digest_b64(&received));
    }

    #[test]
    fn malformed_signing_is_an_invalid_envelope() {
        let keypair = AgentKeypair::generate();
        let did = keypair.public_key().to_did_key();
        let signed = sign("delegation", json!({"orchestrator": did}), &keypair);
        for mutate in [
            |v: &mut Value| v["delegation"]["signing"]["algorithm"] = json!("ES256"),
            |v: &mut Value| {
                v["delegation"]["signing"]["delegation_signature"] = json!("not base64!");
            },
            |v: &mut Value| v["delegation"]["signing"]["extra"] = json!(1),
            |v: &mut Value| v["extra"] = json!(1),
            |v: &mut Value| v["delegation"]["orchestrator"] = json!("did:web:example.com"),
        ] {
            let mut value = signed.clone();
            mutate(&mut value);
            let refused = verified_body("delegation", &value).unwrap_err();
            assert_eq!(refused.reason, RefusalReason::InvalidEnvelope, "{value}");
        }
    }
}
