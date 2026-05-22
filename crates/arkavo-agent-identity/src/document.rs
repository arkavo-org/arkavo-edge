//! The agent identity document and its OpenTDF attribute projection.

use crate::{IdentityError, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

/// OpenTDF attributes that the orchestrator issues — never the identity layer.
///
/// These describe what an agent may *do*; placing them in an identity document
/// would let the AIA make authorization decisions, which is not its role.
pub const ORCHESTRATOR_ISSUED_ATTRIBUTES: [&str; 4] = [
    "agent.tool_access",
    "agent.code_authority",
    "agent.resource_scope",
    "agent.execution_mode",
];

/// Confidence the AIA places in an agent's declared identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// Identity issued without corroborating attestation evidence.
    Unattested,
    /// Identity backed by verified model or trusted-platform attestation.
    Attested,
}

impl TrustLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            TrustLevel::Unattested => "unattested",
            TrustLevel::Attested => "attested",
        }
    }
}

/// Where an agent executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    Local,
    Remote,
}

impl Runtime {
    pub fn as_str(self) -> &'static str {
        match self {
            Runtime::Local => "local",
            Runtime::Remote => "remote",
        }
    }
}

/// A single OpenTDF attribute derived from an identity document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityAttribute {
    pub name: String,
    pub value: String,
}

/// The claims of an agent identity, as issued and signed by the AIA.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityDocument {
    /// Issuing authority identifier, e.g. `agent:id-authority`.
    pub iss: String,
    /// Subject agent identifier, e.g. `agent:worker-17`.
    pub sub: String,
    /// Free-form agent role/type, e.g. `coding`.
    pub agent_type: String,
    /// Agent's Ed25519 public key as a `did:key` string.
    pub public_key: String,
    /// Confidence in the identity (attested vs unattested).
    pub trust_level: TrustLevel,
    /// Where the agent runs.
    pub runtime: Runtime,
    /// Attested model identifier, present when model attestation supplied one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Owning principal — source of `agent.identity.owner`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Owning organization — source of `agent.identity.organization`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    /// Protocols the agent speaks, e.g. `a2a`, `mcp`.
    pub capabilities: Vec<String>,
    /// Issued-at time (RFC 3339).
    pub iat: String,
    /// Expiry time (RFC 3339).
    pub exp: String,
}

impl IdentityDocument {
    /// OpenTDF attributes that originate from this identity.
    ///
    /// Only `agent.identity.*` attributes are emitted — facts about *who* the
    /// agent is. Authorization attributes ([`ORCHESTRATOR_ISSUED_ATTRIBUTES`])
    /// are never produced here.
    pub fn tdf_attributes(&self) -> Vec<IdentityAttribute> {
        let mut attrs = vec![
            IdentityAttribute {
                name: "agent.identity.type".into(),
                value: self.agent_type.clone(),
            },
            IdentityAttribute {
                name: "agent.identity.trust_level".into(),
                value: self.trust_level.as_str().into(),
            },
            IdentityAttribute {
                name: "agent.identity.runtime".into(),
                value: self.runtime.as_str().into(),
            },
        ];
        if let Some(owner) = &self.owner {
            attrs.push(IdentityAttribute {
                name: "agent.identity.owner".into(),
                value: owner.clone(),
            });
        }
        if let Some(organization) = &self.organization {
            attrs.push(IdentityAttribute {
                name: "agent.identity.organization".into(),
                value: organization.clone(),
            });
        }
        attrs
    }

    /// Parse the `exp` claim into a UTC timestamp.
    pub fn expires_at(&self) -> Result<DateTime<Utc>> {
        parse_rfc3339(&self.exp)
    }

    /// Whether the document is expired at `now`.
    pub fn is_expired(&self, now: DateTime<Utc>) -> Result<bool> {
        Ok(now >= self.expires_at()?)
    }
}

pub(crate) fn parse_rfc3339(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| IdentityError::Malformed(format!("invalid RFC 3339 time '{s}': {e}")))
}

pub(crate) fn format_rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> IdentityDocument {
        IdentityDocument {
            iss: "agent:id-authority".into(),
            sub: "agent:worker-17".into(),
            agent_type: "coding".into(),
            public_key: "did:key:zTest".into(),
            trust_level: TrustLevel::Attested,
            runtime: Runtime::Local,
            model: Some("gemma-4".into()),
            owner: Some("user:paul".into()),
            organization: Some("arkavo".into()),
            capabilities: vec!["a2a".into(), "mcp".into()],
            iat: "2026-05-22T00:00:00Z".into(),
            exp: "2026-06-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn tdf_attributes_are_identity_only() {
        let attrs = document().tdf_attributes();
        let names: Vec<&str> = attrs.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"agent.identity.type"));
        assert!(names.contains(&"agent.identity.trust_level"));
        assert!(names.contains(&"agent.identity.runtime"));
        assert!(names.contains(&"agent.identity.owner"));
        assert!(names.contains(&"agent.identity.organization"));
        // The AIA never originates orchestrator-issued authorization attributes.
        for forbidden in ORCHESTRATOR_ISSUED_ATTRIBUTES {
            assert!(!names.contains(&forbidden));
        }
        for attr in &attrs {
            assert!(attr.name.starts_with("agent.identity."));
        }
    }

    #[test]
    fn tdf_attributes_omit_absent_optionals() {
        let mut doc = document();
        doc.owner = None;
        doc.organization = None;
        let names: Vec<String> = doc.tdf_attributes().into_iter().map(|a| a.name).collect();
        assert_eq!(names.len(), 3);
        assert!(!names.iter().any(|n| n == "agent.identity.owner"));
    }

    #[test]
    fn expiry_is_evaluated_against_now() {
        let doc = document();
        let before = parse_rfc3339("2026-05-30T00:00:00Z").unwrap();
        let after = parse_rfc3339("2026-06-02T00:00:00Z").unwrap();
        assert!(!doc.is_expired(before).unwrap());
        assert!(doc.is_expired(after).unwrap());
    }

    #[test]
    fn malformed_exp_is_rejected() {
        let mut doc = document();
        doc.exp = "not-a-timestamp".into();
        assert!(doc.expires_at().is_err());
    }

    #[test]
    fn rfc3339_round_trip_uses_z_suffix() {
        let t = parse_rfc3339("2026-06-01T00:00:00Z").unwrap();
        assert_eq!(format_rfc3339(t), "2026-06-01T00:00:00Z");
    }

    #[test]
    fn document_serializes_to_the_expected_shape() {
        let json = serde_json::to_value(document()).unwrap();
        assert_eq!(json["trust_level"], "attested");
        assert_eq!(json["runtime"], "local");
        assert_eq!(json["capabilities"][0], "a2a");
    }
}
