//! The delegation envelope (spec §7.2).

// Shared by the crate's modules only; plain `pub` here would trip
// `unreachable_pub`.
#![allow(clippy::redundant_pub_crate)]

use arkavo_crypto::{AgentKeypair, AgentPublicKey};
use arkavo_swarmkit::{
    AgentProvisioning, AuthMode, Manifest, RoleSpec, Skill, TdfAttributeReleasePolicy,
};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, SubsecRound, TimeDelta, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::protocol::{DelegationSummary, Refusal, RefusalReason};
use crate::signing;

const MEMBER: &str = "delegation";

/// The longest window an envelope may carry (§7.2).
pub const MAX_LIFETIME: TimeDelta = TimeDelta::days(30);

/// A role handed to one specialist: the envelope's content without its
/// signature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delegation {
    pub orchestrator: String,
    pub specialist: String,
    pub issued: DateTime<Utc>,
    pub expires: DateTime<Utc>,
    pub nonce: String,
    pub flight_id: Uuid,
    pub kit_id: String,
    pub role_id: String,
    pub agent_provisioning: AgentProvisioning,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<Skill>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_grants: Vec<McpGrant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host_requirements: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tdf_policy_role_scoped: Option<TdfAttributeReleasePolicy>,
    #[serde(default)]
    pub context: DelegationContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing_proof: Option<String>,
}

/// What the flight is for, so a specialist enlisted without the manifest
/// knows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_inputs: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peer_roles: Vec<String>,
}

/// A tool grant as issued to the specialist (§8.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpGrant {
    pub server: String,
    pub tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<DateTime<Utc>>,
}

/// Why an orchestrator cannot build or sign an envelope.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BuildError {
    #[error("the kit expired at {0}")]
    KitExpired(DateTime<Utc>),
    #[error("kit.expires is not an RFC 3339 time: {0}")]
    KitExpiry(String),
    #[error("a delegation lasts more than zero and at most 30 days")]
    Lifetime,
    #[error(
        "role {role} grants {server} with auth: delegated, and no token broker issues its tokens"
    )]
    DelegatedGrant { role: String, server: String },
    #[error("the signing key is not the envelope's orchestrator")]
    SignerMismatch,
}

impl Delegation {
    /// The envelope handing `role` of `manifest` to `specialist`, valid from
    /// `issued` for `lifetime` and never past `kit.expires` (C-O7).
    pub fn for_role(
        manifest: &Manifest,
        role: &RoleSpec,
        orchestrator: String,
        specialist: String,
        flight_id: Uuid,
        issued: DateTime<Utc>,
        lifetime: TimeDelta,
    ) -> Result<Self, BuildError> {
        if lifetime <= TimeDelta::zero() || lifetime > MAX_LIFETIME {
            return Err(BuildError::Lifetime);
        }
        let issued = issued.trunc_subsecs(0);
        let mut expires = issued + lifetime;
        if let Some(kit_expires) = &manifest.kit.expires {
            let kit_expires = DateTime::parse_from_rfc3339(kit_expires)
                .map_err(|e| BuildError::KitExpiry(e.to_string()))?
                .with_timezone(&Utc);
            if kit_expires <= issued {
                return Err(BuildError::KitExpired(kit_expires));
            }
            expires = expires.min(kit_expires);
        }
        // No token broker issues delegated tokens, and handing a worker
        // long-lived credentials is what §8.2 steers away from.
        if let Some(grant) = role
            .mcp_tools
            .iter()
            .find(|g| g.auth == AuthMode::Delegated)
        {
            return Err(BuildError::DelegatedGrant {
                role: role.id.clone(),
                server: grant.server.clone(),
            });
        }
        Ok(Self {
            orchestrator,
            specialist,
            issued,
            expires,
            nonce: new_nonce(),
            flight_id,
            kit_id: manifest.kit.id.clone(),
            role_id: role.id.clone(),
            agent_provisioning: role.agent_provisioning.clone(),
            skills: role.skills.clone(),
            mcp_grants: role
                .mcp_tools
                .iter()
                .map(|g| McpGrant {
                    server: g.server.clone(),
                    tools: g.tools.clone(),
                    bearer_token: None,
                    expires: None,
                })
                .collect(),
            host_requirements: role.required_capabilities(),
            tdf_policy_role_scoped: role.tdf_attribute_release_policy.clone(),
            context: DelegationContext {
                objective: Some(manifest.objective.goal.clone()),
                role_description: role.description.clone(),
                initial_inputs: Vec::new(),
                peer_roles: role.handoffs.iter().map(|h| h.to.clone()).collect(),
            },
            pairing_proof: None,
        })
    }

    /// Sign with the orchestrator's key.
    ///
    /// # Panics
    ///
    /// Never: a delegation's fields always encode as JSON.
    pub fn sign(&self, keypair: &AgentKeypair) -> Result<SignedDelegation, BuildError> {
        if keypair.public_key().to_did_key() != self.orchestrator {
            return Err(BuildError::SignerMismatch);
        }
        let body = serde_json::to_value(self).expect("a delegation serializes to JSON");
        let wire = signing::sign(MEMBER, body, keypair);
        Ok(SignedDelegation {
            delegation: self.clone(),
            wire,
        })
    }

    /// What the role tells its agent: the objective and the role's skill
    /// instructions, as an agent started from the kit reads them, or the
    /// role's description when it has no instructions.
    pub fn purpose(&self) -> String {
        let instructions = arkavo_swarmkit::skill_instructions(&self.skills);
        let role = if instructions.is_empty() {
            self.context.role_description.clone().unwrap_or_default()
        } else {
            instructions
        };
        [self.context.objective.clone().unwrap_or_default(), role]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// The tool names the grants allow, sorted, without duplicates.
    pub fn granted_tools(&self) -> Vec<String> {
        let mut tools: Vec<String> = self
            .mcp_grants
            .iter()
            .flat_map(|g| g.tools.iter().cloned())
            .collect();
        tools.sort();
        tools.dedup();
        tools
    }

    pub fn summary(&self) -> DelegationSummary {
        DelegationSummary {
            flight_id: self.flight_id,
            kit_id: self.kit_id.clone(),
            role_id: self.role_id.clone(),
            expires: self.expires,
        }
    }

    /// Check each signed skill against its `did:key` signer (§7.3 step 5).
    /// An unsigned skill passes, as it does on the `-c` kit path; a signer
    /// that is not a `did:key` cannot be checked offline and fails.
    pub(crate) fn verify_skills(&self) -> Result<(), Refusal> {
        for skill in &self.skills {
            let Some(signature) = &skill.signature else {
                continue;
            };
            let unverified = |detail: String| Refusal::new(RefusalReason::SkillUnverified, detail);
            let signer = skill.signed_by.as_deref().unwrap_or_default();
            let key = AgentPublicKey::from_did_key(signer).map_err(|_| {
                unverified(format!(
                    "skill {} is signed by {signer:?}, not a did:key",
                    skill.id
                ))
            })?;
            let payload = skill.payload.as_ref().ok_or_else(|| {
                unverified(format!("skill {} is signed but has no payload", skill.id))
            })?;
            let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| {
                unverified(format!("skill {} signature is not base64url", skill.id))
            })?;
            let digest = blake3::hash(arkavo_swarmkit::canonical_json(payload).as_bytes());
            key.verify(digest.as_bytes(), &signature)
                .map_err(|_| unverified(format!("skill {} signature does not verify", skill.id)))?;
        }
        Ok(())
    }
}

/// A delegation with the signed JSON it arrived or left as.
#[derive(Debug, Clone)]
pub struct SignedDelegation {
    delegation: Delegation,
    wire: Value,
}

impl SignedDelegation {
    /// Parse an envelope and check its signature (§7.3 steps 1 and 2).
    pub fn from_wire(wire: Value) -> Result<Self, Refusal> {
        let body = signing::verified_body(MEMBER, &wire)?;
        let delegation = serde_json::from_value(body)
            .map_err(|e| Refusal::new(RefusalReason::InvalidEnvelope, e.to_string()))?;
        Ok(Self { delegation, wire })
    }

    pub fn delegation(&self) -> &Delegation {
        &self.delegation
    }

    /// The envelope as signed, for `swarmkit/delegate`'s `envelope`.
    pub fn wire(&self) -> &Value {
        &self.wire
    }

    /// The ready acknowledgement's `envelope_blake3` (§7.3.2).
    pub fn envelope_blake3(&self) -> String {
        signing::digest_b64(&self.wire)
    }
}

pub(crate) fn new_nonce() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests;
