//! Purchase authorization, separate from agent authentication.
//!
//! A verified signature says which agent is asking. Whether it may commit this order is a
//! second decision, from evidence issued by someone the merchant trusts to speak for the
//! person: here a `local-test-grant`, a separately signed development consent fixture. It is
//! not AP2 and is never labelled so; AP2 mandates plug in as another [`PurchaseAuthorizer`].

use crate::clock::Clock;
use crate::envelope::{self, SignedEnvelope};
use crate::error::{RejectKind, Rejection};
use crate::evidence::VerifiedAgent;
use crate::keys::Environment;
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::future::Future;

/// The envelope type and label of a development consent grant.
pub const LOCAL_TEST_GRANT: &str = "local-test-grant";

/// What a person authorized one agent to buy from one merchant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalTestGrant {
    /// Always `local-test-grant`.
    pub label: String,
    /// Always `local`: a development grant never authorizes anywhere else.
    pub environment: String,
    pub grant_id: String,
    pub issuer: String,
    /// An opaque reference to the represented person.
    pub principal: String,
    /// The thumbprint of the agent key the grant is issued to.
    pub actor_key_thumbprint: String,
    /// The merchant origin.
    pub merchant: String,
    pub items: Vec<String>,
    pub max_quantity: u32,
    pub currency: String,
    /// The most the agent may spend under this grant, in minor units.
    pub limit_minor: i64,
    pub expires_at: i64,
}

/// One line of an immutable checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckoutLine {
    pub item_id: String,
    pub quantity: u32,
    pub unit_minor: i64,
}

/// A checkout as the merchant computed it. Totals are the server's, never the client's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckoutView {
    pub checkout_id: String,
    pub merchant: String,
    pub lines: Vec<CheckoutLine>,
    pub currency: String,
    pub total_minor: i64,
}

impl CheckoutView {
    /// A digest binding every field, so a decision names exactly what it allowed.
    pub fn hash(&self) -> String {
        let mut h = Sha256::new();
        let mut put = |s: &str| {
            h.update((s.len() as u64).to_be_bytes());
            h.update(s.as_bytes());
        };
        put(&self.checkout_id);
        put(&self.merchant);
        put(&self.currency);
        put(&self.total_minor.to_string());
        for line in &self.lines {
            put(&line.item_id);
            put(&line.quantity.to_string());
            put(&line.unit_minor.to_string());
        }
        crate::keys::hex(&h.finalize())
    }
}

/// What the merchant's ledger knows about a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GrantState {
    pub revoked: bool,
    /// Committed spend under the grant, in minor units.
    pub spent_minor: i64,
}

/// The merchant's record of grant use and revocation.
pub trait GrantLedger: Send + Sync {
    fn grant_state(
        &self,
        grant_id: &str,
    ) -> impl Future<Output = Result<GrantState, String>> + Send;
}

/// An allow decision, bound to the exact checkout it covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationDecision {
    /// The kind of authority: `local-test-grant`.
    pub authority: String,
    pub grant_id: String,
    pub principal: String,
    pub actor_key_thumbprint: String,
    pub checkout_hash: String,
    pub limit_minor: i64,
    /// The decision record of the agent verification it relied on.
    pub verification_decision: String,
}

/// Decides whether a verified agent may commit a checkout.
pub trait PurchaseAuthorizer: Send + Sync {
    fn authorize(
        &self,
        actor: &VerifiedAgent,
        checkout: &CheckoutView,
        evidence: &SignedEnvelope,
        ledger: &impl GrantLedger,
    ) -> impl Future<Output = Result<AuthorizationDecision, Rejection>> + Send;
}

/// Evaluates `local-test-grant` evidence from configured issuers.
pub struct LocalGrantAuthorizer<C> {
    issuers: HashMap<String, VerifyingKey>,
    environment: Environment,
    clock: C,
}

fn denied(why: impl Into<String>) -> Rejection {
    Rejection::new(RejectKind::AuthorityDenied, why)
}

impl<C: Clock> LocalGrantAuthorizer<C> {
    /// `issuers` maps issuer key IDs to their public keys. Grants are only honoured in the
    /// local environment.
    pub fn new(issuers: HashMap<String, VerifyingKey>, environment: Environment, clock: C) -> Self {
        LocalGrantAuthorizer {
            issuers,
            environment,
            clock,
        }
    }

    /// The grant inside `evidence`, if a configured issuer signed it.
    pub fn open(&self, evidence: &SignedEnvelope) -> Result<LocalTestGrant, Rejection> {
        if self.environment != Environment::Local {
            return Err(denied("development grants are not accepted outside local"));
        }
        let key = self
            .issuers
            .get(&evidence.kid)
            .ok_or_else(|| denied("the grant issuer is not trusted"))?;
        let payload = envelope::open(evidence, LOCAL_TEST_GRANT, key).map_err(denied)?;
        let grant: LocalTestGrant = serde_json::from_slice(&payload)
            .map_err(|e| denied(format!("the grant does not parse: {e}")))?;
        if grant.label != LOCAL_TEST_GRANT || grant.environment != Environment::Local.name() {
            return Err(denied("not a local-test-grant"));
        }
        if grant.issuer != evidence.kid {
            return Err(denied("the grant names another issuer"));
        }
        Ok(grant)
    }

    /// Checks `grant` against the actor, the checkout and the ledger state.
    pub fn evaluate(
        &self,
        grant: &LocalTestGrant,
        state: GrantState,
        actor: &VerifiedAgent,
        checkout: &CheckoutView,
    ) -> Result<AuthorizationDecision, Rejection> {
        if state.revoked {
            return Err(denied("the grant is revoked"));
        }
        if self.clock.now() >= grant.expires_at {
            return Err(denied("the grant has expired"));
        }
        if grant.actor_key_thumbprint != actor.key_thumbprint() {
            return Err(denied("the grant is issued to another agent"));
        }
        if grant.merchant != checkout.merchant {
            return Err(denied("the grant is for another merchant"));
        }
        if grant.currency != checkout.currency {
            return Err(denied("the checkout currency differs from the grant"));
        }
        for line in &checkout.lines {
            if !grant.items.contains(&line.item_id) {
                return Err(denied(format!(
                    "{} is not covered by the grant",
                    line.item_id
                )));
            }
            if line.quantity > grant.max_quantity {
                return Err(denied("the quantity exceeds the grant"));
            }
        }
        let after = state.spent_minor.checked_add(checkout.total_minor);
        if checkout.total_minor <= 0 || after.is_none_or(|t| t > grant.limit_minor) {
            return Err(denied("the total exceeds the grant's remaining limit"));
        }
        Ok(AuthorizationDecision {
            authority: LOCAL_TEST_GRANT.to_owned(),
            grant_id: grant.grant_id.clone(),
            principal: grant.principal.clone(),
            actor_key_thumbprint: grant.actor_key_thumbprint.clone(),
            checkout_hash: checkout.hash(),
            limit_minor: grant.limit_minor,
            verification_decision: actor.decision_id().to_owned(),
        })
    }
}

impl<C: Clock> PurchaseAuthorizer for LocalGrantAuthorizer<C> {
    async fn authorize(
        &self,
        actor: &VerifiedAgent,
        checkout: &CheckoutView,
        evidence: &SignedEnvelope,
        ledger: &impl GrantLedger,
    ) -> Result<AuthorizationDecision, Rejection> {
        let grant = self.open(evidence)?;
        let state = ledger
            .grant_state(&grant.grant_id)
            .await
            .map_err(|e| Rejection::new(RejectKind::Unavailable, e))?;
        self.evaluate(&grant, state, actor, checkout)
    }
}
