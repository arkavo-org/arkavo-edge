//! Revoking a delegation, or releasing a binding (spec §7.4).

use arkavo_crypto::AgentKeypair;
use chrono::{DateTime, SubsecRound, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::envelope::{BuildError, new_nonce};
use crate::protocol::{Refusal, RefusalReason};
use crate::signing;

const MEMBER: &str = "revocation";

/// An orchestrator's revocation, without its signature. A `flight_id` of
/// `None` revokes whatever delegation the specialist holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revocation {
    pub orchestrator: String,
    pub specialist: String,
    pub issued: DateTime<Utc>,
    pub nonce: String,
    pub flight_id: Option<Uuid>,
    pub release_binding: bool,
}

impl Revocation {
    pub fn new(
        orchestrator: String,
        specialist: String,
        flight_id: Option<Uuid>,
        release_binding: bool,
        issued: DateTime<Utc>,
    ) -> Self {
        Self {
            orchestrator,
            specialist,
            issued: issued.trunc_subsecs(0),
            nonce: new_nonce(),
            flight_id,
            release_binding,
        }
    }

    /// Sign with the orchestrator's key.
    ///
    /// # Panics
    ///
    /// Never: a revocation's fields always encode as JSON.
    pub fn sign(&self, keypair: &AgentKeypair) -> Result<SignedRevocation, BuildError> {
        if keypair.public_key().to_did_key() != self.orchestrator {
            return Err(BuildError::SignerMismatch);
        }
        let body = serde_json::to_value(self).expect("a revocation serializes to JSON");
        Ok(SignedRevocation {
            revocation: self.clone(),
            wire: signing::sign(MEMBER, body, keypair),
        })
    }
}

/// A revocation with the signed JSON it arrived or left as.
#[derive(Debug, Clone)]
pub struct SignedRevocation {
    revocation: Revocation,
    wire: Value,
}

impl SignedRevocation {
    pub fn from_wire(wire: Value) -> Result<Self, Refusal> {
        let body = signing::verified_body(MEMBER, &wire)?;
        let revocation = serde_json::from_value(body)
            .map_err(|e| Refusal::new(RefusalReason::InvalidEnvelope, e.to_string()))?;
        Ok(Self { revocation, wire })
    }

    pub fn revocation(&self) -> &Revocation {
        &self.revocation
    }

    /// The revocation as signed, for `swarmkit/revoke`'s `revocation`.
    pub fn wire(&self) -> &Value {
        &self.wire
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[arkavo_test_macros::spec("SKD-005")]
    #[test]
    fn a_revocation_round_trips_and_names_a_null_flight() {
        let keypair = AgentKeypair::generate();
        let did = keypair.public_key().to_did_key();
        let revocation =
            Revocation::new(did, "did:key:z6Mkspecialist".into(), None, true, Utc::now());
        let signed = revocation.sign(&keypair).unwrap();
        assert!(signed.wire()["revocation"]["flight_id"].is_null());
        let text = serde_json::to_string(signed.wire()).unwrap();
        let received = SignedRevocation::from_wire(serde_json::from_str(&text).unwrap()).unwrap();
        assert_eq!(received.revocation(), &revocation);
        assert_eq!(
            revocation.sign(&AgentKeypair::generate()).unwrap_err(),
            BuildError::SignerMismatch
        );
    }
}
