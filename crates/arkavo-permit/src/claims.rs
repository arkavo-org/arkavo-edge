//! Permit claim set: standard CWT claims plus Arkavo private-use claims.
//!
//! Claim labels are defined in `docs/permit-cwt-schema.md`. Private-use
//! labels live below -65536 per RFC 8392 section 4.

use crate::canonical::argument_hash;
use crate::error::PermitError;
use crate::hash::HashAlgorithm;
use ciborium::value::{Integer, Value};
use coset::{AsCborValue, CoseKey};

// Standard CWT claims (RFC 8392) and confirmation (RFC 8747).
pub const CLAIM_ISSUER: i64 = 1;
pub const CLAIM_SUBJECT: i64 = 2;
pub const CLAIM_EXPIRATION: i64 = 4;
pub const CLAIM_NOT_BEFORE: i64 = 5;
pub const CLAIM_ISSUED_AT: i64 = 6;
pub const CLAIM_CONFIRMATION: i64 = 8;

// Arkavo private-use claims.
pub const CLAIM_AGENT_WORKLOAD_ID: i64 = -70001;
pub const CLAIM_POLICY_BUNDLE_HASH: i64 = -70002;
pub const CLAIM_TOOL_NAME: i64 = -70003;
pub const CLAIM_ARGUMENT_HASH: i64 = -70004;
pub const CLAIM_DATA_CLASSIFICATIONS: i64 = -70005;
pub const CLAIM_BUDGET: i64 = -70006;
pub const CLAIM_SEQUENCE_STATE_HASH: i64 = -70007;
pub const CLAIM_PARENT_PERMIT: i64 = -70008;

// Keys inside the budget map (claim -70006).
pub const BUDGET_MAX_INVOCATIONS: i64 = 1;
pub const BUDGET_TOKEN_CEILING: i64 = 2;
pub const BUDGET_COST_MICRO_USD: i64 = 3;

// Key inside the cnf map (claim 8) per RFC 8747: 1 = COSE_Key.
pub const CNF_COSE_KEY: i64 = 1;

/// Execution budget bound into the permit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Budget {
    /// Maximum number of tool invocations this permit authorizes.
    pub max_invocations: u64,
    /// Optional ceiling on total tokens consumed across invocations.
    pub token_ceiling: Option<u64>,
    /// Optional cost ceiling in micro-USD (10^-6 USD).
    pub cost_micro_usd: Option<u64>,
}

impl Budget {
    fn validate(&self) -> Result<(), PermitError> {
        if self.max_invocations == 0 {
            return Err(PermitError::MalformedClaim(
                "budget.max_invocations is zero",
            ));
        }
        Ok(())
    }

    fn to_value(&self) -> Value {
        let mut entries = vec![int_pair(BUDGET_MAX_INVOCATIONS, self.max_invocations)];
        if let Some(tokens) = self.token_ceiling {
            entries.push(int_pair(BUDGET_TOKEN_CEILING, tokens));
        }
        if let Some(cost) = self.cost_micro_usd {
            entries.push(int_pair(BUDGET_COST_MICRO_USD, cost));
        }
        Value::Map(entries)
    }

    fn from_value(value: &Value) -> Result<Self, PermitError> {
        let map = as_map(value, "budget")?;
        let raw = take_int(map, BUDGET_MAX_INVOCATIONS, "budget.max_invocations")?;
        let max_invocations = u64::try_from(raw)
            .map_err(|_| PermitError::MalformedClaim("budget.max_invocations is negative"))?;
        let budget = Self {
            max_invocations,
            token_ceiling: get_u64(map, BUDGET_TOKEN_CEILING, "budget.token_ceiling")?,
            cost_micro_usd: get_u64(map, BUDGET_COST_MICRO_USD, "budget.cost_micro_usd")?,
        };
        budget.validate()?;
        Ok(budget)
    }
}

/// Claims carried by a permit CWT. All fields except `parent_permit` are
/// required; the `cnf` confirmation claim is handled separately because it is
/// derived from the signing key at mint time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermitClaims {
    /// iss: permit issuer identity (URI or DID).
    pub issuer: String,
    /// sub: delegator / human identity on whose behalf the agent acts.
    pub subject: String,
    /// Agent workload identity (SPIFFE-style workload ID).
    pub agent_workload_id: String,
    /// Hash of the policy bundle authorizing this permit.
    pub policy_bundle_hash: Vec<u8>,
    /// Fully-qualified tool name this permit authorizes.
    pub tool_name: String,
    /// Hash of the canonicalized tool arguments (see `canonical_json`).
    pub argument_hash: Vec<u8>,
    /// TDF data-classification attributes the tool may touch.
    pub data_classifications: Vec<String>,
    /// Execution budget bound into the permit.
    pub budget: Budget,
    /// Hash of the sequence state (replay/ordering anchor).
    pub sequence_state_hash: Vec<u8>,
    /// iat: seconds since UNIX epoch.
    pub issued_at: i64,
    /// nbf: seconds since UNIX epoch.
    pub not_before: i64,
    /// exp: seconds since UNIX epoch.
    pub expires_at: i64,
    /// Hash of the parent permit's CWT bytes, for A2A delegation chains.
    pub parent_permit: Option<Vec<u8>>,
}

impl PermitClaims {
    /// Structural validation, independent of signature and wall-clock time.
    pub fn validate(&self) -> Result<(), PermitError> {
        for (value, name) in [
            (&self.issuer, "iss"),
            (&self.subject, "sub"),
            (&self.agent_workload_id, "agent_workload_id"),
            (&self.tool_name, "tool_name"),
        ] {
            if value.is_empty() {
                return Err(PermitError::MalformedClaim(name));
            }
        }
        // SHA-256 and BLAKE3 default outputs are 32 bytes. Reject any other
        // length at validate time so a 1-byte hash cannot pass as well-formed.
        const DIGEST_LEN: usize = 32;
        for (value, name) in [
            (&self.policy_bundle_hash, "policy_bundle_hash"),
            (&self.argument_hash, "argument_hash"),
            (&self.sequence_state_hash, "sequence_state_hash"),
        ] {
            if value.len() != DIGEST_LEN {
                return Err(PermitError::MalformedClaim(name));
            }
        }
        for classification in &self.data_classifications {
            if classification.is_empty() {
                return Err(PermitError::MalformedClaim("data_classifications"));
            }
        }
        if let Some(parent) = &self.parent_permit
            && parent.len() != DIGEST_LEN
        {
            return Err(PermitError::MalformedClaim("parent_permit"));
        }
        self.budget.validate()?;
        if self.not_before >= self.expires_at {
            return Err(PermitError::MalformedClaim("nbf is not before exp"));
        }
        Ok(())
    }

    /// Check that a tool invocation matches this permit's binding.
    pub fn verify_invocation(
        &self,
        tool_name: &str,
        arguments: &serde_json::Value,
        algorithm: HashAlgorithm,
    ) -> Result<(), PermitError> {
        if self.tool_name != tool_name {
            return Err(PermitError::BindingMismatch(format!(
                "tool name {} is not {}",
                tool_name, self.tool_name
            )));
        }
        let hash = argument_hash(arguments, algorithm);
        if hash != self.argument_hash {
            return Err(PermitError::BindingMismatch(
                "canonical argument hash mismatch".to_string(),
            ));
        }
        Ok(())
    }

    /// Encode the claims (plus the confirmation key) as a CBOR value.
    pub fn to_cbor_value(&self, confirmation_key: &CoseKey) -> Result<Value, PermitError> {
        let cnf_key_value = confirmation_key
            .clone()
            .to_cbor_value()
            .map_err(|e| PermitError::CborSerialize(format!("cnf COSE_Key: {e}")))?;
        let cnf = Value::Map(vec![(int_value(CNF_COSE_KEY), cnf_key_value)]);
        let classifications = Value::Array(
            self.data_classifications
                .iter()
                .map(|c| Value::Text(c.clone()))
                .collect(),
        );
        let mut entries = vec![
            (int_value(CLAIM_ISSUER), Value::Text(self.issuer.clone())),
            (int_value(CLAIM_SUBJECT), Value::Text(self.subject.clone())),
            int_pair(CLAIM_EXPIRATION, self.expires_at),
            int_pair(CLAIM_NOT_BEFORE, self.not_before),
            int_pair(CLAIM_ISSUED_AT, self.issued_at),
            (int_value(CLAIM_CONFIRMATION), cnf),
            (
                int_value(CLAIM_AGENT_WORKLOAD_ID),
                Value::Text(self.agent_workload_id.clone()),
            ),
            (
                int_value(CLAIM_POLICY_BUNDLE_HASH),
                Value::Bytes(self.policy_bundle_hash.clone()),
            ),
            (
                int_value(CLAIM_TOOL_NAME),
                Value::Text(self.tool_name.clone()),
            ),
            (
                int_value(CLAIM_ARGUMENT_HASH),
                Value::Bytes(self.argument_hash.clone()),
            ),
            (int_value(CLAIM_DATA_CLASSIFICATIONS), classifications),
            (int_value(CLAIM_BUDGET), self.budget.to_value()),
            (
                int_value(CLAIM_SEQUENCE_STATE_HASH),
                Value::Bytes(self.sequence_state_hash.clone()),
            ),
        ];
        if let Some(parent) = &self.parent_permit {
            entries.push((int_value(CLAIM_PARENT_PERMIT), Value::Bytes(parent.clone())));
        }
        Ok(Value::Map(entries))
    }

    /// Parse a claims-map CBOR value, returning the claims and the
    /// confirmation key. Fails closed on any malformed required claim;
    /// unknown claims are ignored for forward compatibility.
    pub fn from_cbor_value(value: &Value) -> Result<(Self, CoseKey), PermitError> {
        let map = as_map(value, "claims set")?;
        let classifications = match take(map, CLAIM_DATA_CLASSIFICATIONS, "data_classifications")? {
            Value::Array(items) => items
                .iter()
                .map(|item| {
                    item.as_text()
                        .map(str::to_string)
                        .ok_or(PermitError::MalformedClaim("data_classifications entry"))
                })
                .collect::<Result<Vec<String>, PermitError>>()?,
            _ => return Err(PermitError::MalformedClaim("data_classifications")),
        };
        let cnf_map = as_map(
            take(map, CLAIM_CONFIRMATION, "cnf")?,
            "cnf confirmation map",
        )?;
        let cnf_key_value = take(cnf_map, CNF_COSE_KEY, "cnf COSE_Key")?;
        let confirmation_key = CoseKey::from_cbor_value(cnf_key_value.clone())
            .map_err(|e| PermitError::InvalidConfirmationKey(format!("COSE_Key parse: {e}")))?;
        let parent_permit = match get(map, CLAIM_PARENT_PERMIT) {
            Some(Value::Bytes(bytes)) => Some(bytes.clone()),
            Some(_) => return Err(PermitError::MalformedClaim("parent_permit")),
            None => None,
        };
        let claims = Self {
            issuer: take_text(map, CLAIM_ISSUER, "iss")?,
            subject: take_text(map, CLAIM_SUBJECT, "sub")?,
            agent_workload_id: take_text(map, CLAIM_AGENT_WORKLOAD_ID, "agent_workload_id")?,
            policy_bundle_hash: take_bytes(map, CLAIM_POLICY_BUNDLE_HASH, "policy_bundle_hash")?,
            tool_name: take_text(map, CLAIM_TOOL_NAME, "tool_name")?,
            argument_hash: take_bytes(map, CLAIM_ARGUMENT_HASH, "argument_hash")?,
            data_classifications: classifications,
            budget: Budget::from_value(take(map, CLAIM_BUDGET, "budget")?)?,
            sequence_state_hash: take_bytes(map, CLAIM_SEQUENCE_STATE_HASH, "sequence_state_hash")?,
            issued_at: take_int(map, CLAIM_ISSUED_AT, "iat")?,
            not_before: take_int(map, CLAIM_NOT_BEFORE, "nbf")?,
            expires_at: take_int(map, CLAIM_EXPIRATION, "exp")?,
            parent_permit,
        };
        claims.validate()?;
        Ok((claims, confirmation_key))
    }
}

fn int_value(key: i64) -> Value {
    Value::Integer(Integer::from(key))
}

fn int_pair<V: Into<Integer>>(key: i64, value: V) -> (Value, Value) {
    (int_value(key), Value::Integer(value.into()))
}

fn as_map<'a>(value: &'a Value, name: &'static str) -> Result<&'a [(Value, Value)], PermitError> {
    match value {
        Value::Map(entries) => Ok(entries),
        _ => Err(PermitError::MalformedClaim(name)),
    }
}

fn get(map: &[(Value, Value)], key: i64) -> Option<&Value> {
    let key = Integer::from(key);
    map.iter()
        .find(|(k, _)| matches!(k, Value::Integer(i) if *i == key))
        .map(|(_, v)| v)
}

fn take<'a>(
    map: &'a [(Value, Value)],
    key: i64,
    name: &'static str,
) -> Result<&'a Value, PermitError> {
    get(map, key).ok_or(PermitError::MissingClaim(name))
}

fn take_text(map: &[(Value, Value)], key: i64, name: &'static str) -> Result<String, PermitError> {
    match take(map, key, name)? {
        Value::Text(text) => Ok(text.clone()),
        _ => Err(PermitError::MalformedClaim(name)),
    }
}

fn take_bytes(
    map: &[(Value, Value)],
    key: i64,
    name: &'static str,
) -> Result<Vec<u8>, PermitError> {
    match take(map, key, name)? {
        Value::Bytes(bytes) => Ok(bytes.clone()),
        _ => Err(PermitError::MalformedClaim(name)),
    }
}

fn take_int(map: &[(Value, Value)], key: i64, name: &'static str) -> Result<i64, PermitError> {
    match take(map, key, name)? {
        Value::Integer(i) => i64::try_from(*i).map_err(|_| PermitError::MalformedClaim(name)),
        _ => Err(PermitError::MalformedClaim(name)),
    }
}

fn get_u64(
    map: &[(Value, Value)],
    key: i64,
    name: &'static str,
) -> Result<Option<u64>, PermitError> {
    match get(map, key) {
        Some(Value::Integer(i)) => {
            let raw = i128::from(*i);
            u64::try_from(raw)
                .map(Some)
                .map_err(|_| PermitError::MalformedClaim(name))
        }
        Some(_) => Err(PermitError::MalformedClaim(name)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::PermitSigner;
    use arkavo_crypto::AgentKeypair;

    fn sample_claims() -> PermitClaims {
        PermitClaims {
            issuer: "https://issuer.example".to_string(),
            subject: "did:example:alice".to_string(),
            agent_workload_id: "spiffe://edge/agent-1".to_string(),
            policy_bundle_hash: vec![1; 32],
            tool_name: "arkavo.fs.read".to_string(),
            argument_hash: vec![2; 32],
            data_classifications: vec!["tdf:confidential".to_string()],
            budget: Budget {
                max_invocations: 3,
                token_ceiling: Some(10_000),
                cost_micro_usd: None,
            },
            sequence_state_hash: vec![3; 32],
            issued_at: 1_000,
            not_before: 1_000,
            expires_at: 1_300,
            parent_permit: Some(vec![9; 32]),
        }
    }

    fn sample_key() -> CoseKey {
        PermitSigner::Ed25519(AgentKeypair::generate()).cose_key()
    }

    #[test]
    fn claims_cbor_roundtrip() {
        let claims = sample_claims();
        let key = sample_key();
        let value = claims.to_cbor_value(&key).unwrap();
        let (decoded, decoded_key) = PermitClaims::from_cbor_value(&value).unwrap();
        assert_eq!(claims, decoded);
        assert_eq!(
            key.to_cbor_value().unwrap(),
            decoded_key.to_cbor_value().unwrap()
        );
    }

    #[test]
    fn missing_required_claim_fails_closed() {
        let claims = sample_claims();
        let key = sample_key();
        let value = claims.to_cbor_value(&key).unwrap();
        let Value::Map(entries) = value else { panic!() };
        let stripped: Vec<(Value, Value)> = entries
            .into_iter()
            .filter(|(k, _)| k != &int_value(CLAIM_TOOL_NAME))
            .collect();
        let result = PermitClaims::from_cbor_value(&Value::Map(stripped));
        assert!(matches!(
            result,
            Err(PermitError::MissingClaim("tool_name"))
        ));
    }

    #[test]
    fn wrong_claim_type_fails_closed() {
        let claims = sample_claims();
        let key = sample_key();
        let value = claims.to_cbor_value(&key).unwrap();
        let Value::Map(mut entries) = value else {
            panic!()
        };
        for (k, v) in &mut entries {
            if k == &int_value(CLAIM_ARGUMENT_HASH) {
                *v = Value::Text("not bytes".to_string());
            }
        }
        let result = PermitClaims::from_cbor_value(&Value::Map(entries));
        assert!(matches!(
            result,
            Err(PermitError::MalformedClaim("argument_hash"))
        ));
    }

    #[test]
    fn invalid_temporal_order_rejected() {
        let mut claims = sample_claims();
        claims.not_before = claims.expires_at;
        assert!(claims.validate().is_err());
    }

    #[test]
    fn zero_invocation_budget_rejected() {
        let mut claims = sample_claims();
        claims.budget.max_invocations = 0;
        assert!(claims.validate().is_err());
    }

    #[test]
    fn empty_required_strings_rejected() {
        let mut claims = sample_claims();
        claims.tool_name = String::new();
        assert!(claims.validate().is_err());
    }

    #[test]
    fn digest_claims_must_be_32_bytes() {
        let mut claims = sample_claims();
        claims.policy_bundle_hash = vec![1];
        assert!(matches!(
            claims.validate(),
            Err(PermitError::MalformedClaim("policy_bundle_hash"))
        ));

        claims = sample_claims();
        claims.argument_hash = vec![1; 31];
        assert!(matches!(
            claims.validate(),
            Err(PermitError::MalformedClaim("argument_hash"))
        ));

        claims = sample_claims();
        claims.sequence_state_hash = vec![1; 33];
        assert!(matches!(
            claims.validate(),
            Err(PermitError::MalformedClaim("sequence_state_hash"))
        ));

        claims = sample_claims();
        claims.parent_permit = Some(vec![1]);
        assert!(matches!(
            claims.validate(),
            Err(PermitError::MalformedClaim("parent_permit"))
        ));
    }

    #[test]
    fn verify_invocation_checks_tool_and_arguments() {
        let arguments = serde_json::json!({"b": 1, "a": "x"});
        let mut claims = sample_claims();
        claims.tool_name = "arkavo.fs.read".to_string();
        claims.argument_hash = argument_hash(&arguments, HashAlgorithm::Sha256);

        // Same arguments in a different key order must match.
        let reordered = serde_json::json!({"a": "x", "b": 1});
        assert!(
            claims
                .verify_invocation("arkavo.fs.read", &reordered, HashAlgorithm::Sha256)
                .is_ok()
        );

        assert!(matches!(
            claims.verify_invocation("arkavo.fs.write", &arguments, HashAlgorithm::Sha256),
            Err(PermitError::BindingMismatch(_))
        ));
        let different_args = serde_json::json!({"a": "x", "b": 2});
        assert!(matches!(
            claims.verify_invocation("arkavo.fs.read", &different_args, HashAlgorithm::Sha256),
            Err(PermitError::BindingMismatch(_))
        ));
    }
}
