//! Mutual proof of possession for `agent/exchangeKeys`.
//!
//! A gossip key binds an agent id to the key that signs its announcements and
//! votes. Before this, any caller could bind or replace any id's key without
//! holding it, then sign gossip as that agent. Now each side signs for its
//! own key: the caller over its id, its key, the recipient's id and a fresh
//! challenge; the recipient over its id, its key and that challenge. A replayed
//! request can only re-assert a key the replayer does not hold, and the
//! registry refuses to replace a bound key, so no nonce state is kept.

use crate::{GossipError, GossipResult};
use arkavo_crypto::{AgentKeypair, AgentPublicKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};

const REQUEST_DOMAIN: &[u8] = b"arkavo/gossip/exchange-keys/request/v1";
const RESPONSE_DOMAIN: &[u8] = b"arkavo/gossip/exchange-keys/response/v1";
const CHALLENGE_LEN: usize = 32;

/// What a caller sends: its id and key, the id it addresses, a challenge for
/// the recipient to sign, and its signature over all of them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyExchangeRequest {
    pub peer_id: String,
    pub public_key: String,
    pub recipient: String,
    pub challenge: String,
    pub signature: String,
}

/// What the recipient returns: its key and its signature over the caller's
/// challenge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyExchangeResponse {
    pub public_key: String,
    pub signature: String,
}

/// Fields joined with their lengths, so no two field lists sign the same bytes.
fn signed_bytes(domain: &[u8], fields: &[&[u8]]) -> Vec<u8> {
    let mut out =
        Vec::with_capacity(domain.len() + fields.iter().map(|f| f.len() + 8).sum::<usize>());
    out.extend_from_slice(domain);
    for field in fields {
        out.extend_from_slice(&(field.len() as u64).to_be_bytes());
        out.extend_from_slice(field);
    }
    out
}

fn decode_hex(field: &str, value: &str) -> GossipResult<Vec<u8>> {
    hex::decode(value).map_err(|_| GossipError::Verification(format!("{field} is not hex")))
}

fn parse_key(value: &str) -> GossipResult<AgentPublicKey> {
    AgentPublicKey::from_base64(value)
        .map_err(|_| GossipError::Verification("public_key is not a valid key".into()))
}

/// Build a request from `peer_id` to `recipient`. Returns the request and the
/// challenge the reply must sign.
pub fn sign_request(
    keypair: &AgentKeypair,
    peer_id: &str,
    recipient: &str,
) -> (KeyExchangeRequest, Vec<u8>) {
    let mut challenge = vec![0u8; CHALLENGE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut challenge);
    let public_key = keypair.public_key().to_base64();
    let signature = keypair.sign(&signed_bytes(
        REQUEST_DOMAIN,
        &[
            peer_id.as_bytes(),
            public_key.as_bytes(),
            recipient.as_bytes(),
            &challenge,
        ],
    ));
    let request = KeyExchangeRequest {
        peer_id: peer_id.to_string(),
        public_key,
        recipient: recipient.to_string(),
        challenge: hex::encode(&challenge),
        signature: hex::encode(signature),
    };
    (request, challenge)
}

/// Check a request addressed to `our_id` and return the caller's proven key.
pub fn verify_request(request: &KeyExchangeRequest, our_id: &str) -> GossipResult<AgentPublicKey> {
    if request.recipient != our_id {
        return Err(GossipError::Verification(
            "request is addressed to another agent".into(),
        ));
    }
    let challenge = decode_hex("challenge", &request.challenge)?;
    if challenge.len() != CHALLENGE_LEN {
        return Err(GossipError::Verification(
            "challenge has the wrong length".into(),
        ));
    }
    let key = parse_key(&request.public_key)?;
    let signature = decode_hex("signature", &request.signature)?;
    key.verify(
        &signed_bytes(
            REQUEST_DOMAIN,
            &[
                request.peer_id.as_bytes(),
                request.public_key.as_bytes(),
                request.recipient.as_bytes(),
                &challenge,
            ],
        ),
        &signature,
    )?;
    Ok(key)
}

/// The recipient's reply to a verified request.
pub fn sign_response(
    keypair: &AgentKeypair,
    our_id: &str,
    request: &KeyExchangeRequest,
) -> GossipResult<KeyExchangeResponse> {
    let challenge = decode_hex("challenge", &request.challenge)?;
    let public_key = keypair.public_key().to_base64();
    let signature = keypair.sign(&signed_bytes(
        RESPONSE_DOMAIN,
        &[
            our_id.as_bytes(),
            public_key.as_bytes(),
            request.peer_id.as_bytes(),
            &challenge,
        ],
    ));
    Ok(KeyExchangeResponse {
        public_key,
        signature: hex::encode(signature),
    })
}

/// Check the reply from `responder` to the request `peer_id` sent with
/// `challenge`, and return the responder's proven key.
pub fn verify_response(
    response: &KeyExchangeResponse,
    responder: &str,
    peer_id: &str,
    challenge: &[u8],
) -> GossipResult<AgentPublicKey> {
    let key = parse_key(&response.public_key)?;
    let signature = decode_hex("signature", &response.signature)?;
    key.verify(
        &signed_bytes(
            RESPONSE_DOMAIN,
            &[
                responder.as_bytes(),
                response.public_key.as_bytes(),
                peer_id.as_bytes(),
                challenge,
            ],
        ),
        &signature,
    )?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("INGRESS-004")]
    #[test]
    fn a_proven_request_and_reply_verify() {
        let caller = AgentKeypair::generate();
        let callee = AgentKeypair::generate();
        let (request, challenge) = sign_request(&caller, "alpha", "beta");

        let caller_key = verify_request(&request, "beta").unwrap();
        assert_eq!(caller_key.to_bytes(), caller.public_key().to_bytes());

        let response = sign_response(&callee, "beta", &request).unwrap();
        let callee_key = verify_response(&response, "beta", "alpha", &challenge).unwrap();
        assert_eq!(callee_key.to_bytes(), callee.public_key().to_bytes());
    }

    /// Regression: the old request named a key the caller need not hold.
    #[spec("INGRESS-004")]
    #[test]
    fn a_key_the_caller_does_not_hold_is_refused() {
        let caller = AgentKeypair::generate();
        let victim = AgentKeypair::generate();
        let (mut request, _) = sign_request(&caller, "alpha", "beta");
        request.public_key = victim.public_key().to_base64();
        assert!(verify_request(&request, "beta").is_err());
    }

    #[spec("INGRESS-004")]
    #[test]
    fn a_request_for_another_recipient_is_refused() {
        let caller = AgentKeypair::generate();
        let (request, _) = sign_request(&caller, "alpha", "beta");
        assert!(verify_request(&request, "gamma").is_err());

        let mut retargeted = request.clone();
        retargeted.recipient = "gamma".into();
        assert!(verify_request(&retargeted, "gamma").is_err());
    }

    #[spec("INGRESS-004")]
    #[test]
    fn a_changed_id_or_challenge_is_refused() {
        let caller = AgentKeypair::generate();
        let (request, _) = sign_request(&caller, "alpha", "beta");

        let mut renamed = request.clone();
        renamed.peer_id = "trusted-peer".into();
        assert!(verify_request(&renamed, "beta").is_err());

        let mut rechallenged = request.clone();
        rechallenged.challenge = hex::encode([7u8; CHALLENGE_LEN]);
        assert!(verify_request(&rechallenged, "beta").is_err());

        let mut short = request;
        short.challenge = hex::encode([7u8; 4]);
        assert!(verify_request(&short, "beta").is_err());
    }

    /// The caller also stopped trusting an unproven reply key.
    #[spec("INGRESS-004")]
    #[test]
    fn a_reply_must_sign_this_challenge_with_its_own_key() {
        let caller = AgentKeypair::generate();
        let callee = AgentKeypair::generate();
        let impostor = AgentKeypair::generate();
        let (request, challenge) = sign_request(&caller, "alpha", "beta");
        let response = sign_response(&callee, "beta", &request).unwrap();

        let mut swapped = response.clone();
        swapped.public_key = impostor.public_key().to_base64();
        assert!(verify_response(&swapped, "beta", "alpha", &challenge).is_err());

        let (_, other_challenge) = sign_request(&caller, "alpha", "beta");
        assert!(verify_response(&response, "beta", "alpha", &other_challenge).is_err());
        assert!(verify_response(&response, "delta", "alpha", &challenge).is_err());
    }
}
