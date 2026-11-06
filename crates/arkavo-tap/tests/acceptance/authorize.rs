//! T17, T18, T19, T24: purchase authority is separate evidence, checked against the verified
//! actor and the exact checkout.

use crate::common::*;
use arkavo_tap::*;
use ed25519_dalek::SigningKey;
use std::collections::HashMap;
use std::future::Future;

const ISSUER: &str = "consent-issuer";

fn issuer_key() -> SigningKey {
    SigningKey::from_bytes(&[42u8; 32])
}

fn grant(actor: &KeyRecord) -> LocalTestGrant {
    LocalTestGrant {
        label: LOCAL_TEST_GRANT.into(),
        environment: "local".into(),
        grant_id: "grant-1".into(),
        issuer: ISSUER.into(),
        principal: "principal:alice".into(),
        actor_key_thumbprint: actor.thumbprint(),
        merchant: MERCHANT.into(),
        items: vec!["demo-001".into()],
        max_quantity: 2,
        currency: "USD".into(),
        limit_minor: 5_000,
        expires_at: NOW + 3_600,
    }
}

fn seal(g: &LocalTestGrant) -> SignedEnvelope {
    arkavo_tap::envelope::seal(
        LOCAL_TEST_GRANT,
        ISSUER,
        &issuer_key(),
        &serde_json::to_vec(g).unwrap(),
    )
}

fn checkout(quantity: u32) -> CheckoutView {
    CheckoutView {
        checkout_id: "co-1".into(),
        merchant: MERCHANT.into(),
        lines: vec![CheckoutLine {
            item_id: "demo-001".into(),
            quantity,
            unit_minor: 1_250,
        }],
        currency: "USD".into(),
        total_minor: 1_250 * quantity as i64,
    }
}

struct Ledger(GrantState);

impl GrantLedger for Ledger {
    fn grant_state(
        &self,
        _grant_id: &str,
    ) -> impl Future<Output = Result<GrantState, String>> + Send {
        std::future::ready(Ok(self.0))
    }
}

struct DownLedger;

impl GrantLedger for DownLedger {
    fn grant_state(
        &self,
        _grant_id: &str,
    ) -> impl Future<Output = Result<GrantState, String>> + Send {
        std::future::ready(Err("ledger unavailable".into()))
    }
}

fn authorizer(env: Environment, clock: ManualClock) -> LocalGrantAuthorizer<ManualClock> {
    let issuers = HashMap::from([(ISSUER.to_owned(), issuer_key().verifying_key())]);
    LocalGrantAuthorizer::new(issuers, env, clock)
}

/// A verified commit request from `seed`'s agent.
async fn verified(seed: u8, key_id: &str) -> (VerifiedAgent, KeyRecord) {
    let (signer, rec) = agent(seed, key_id, KeyStatus::Active);
    let dir = tempfile::tempdir().unwrap();
    let v = verifier_at(
        &dir.path().join("r.sqlite"),
        StaticResolver::new(vec![rec.clone()]),
        ManualClock::new(NOW),
        authority(),
    )
    .await;
    let body = br#"{"checkoutId":"co-1"}"#;
    let ctx = sign(
        &signer,
        &write("/demo/v1/orders", body, "order-1"),
        Operation::Commit,
        Purpose::Pay,
        NOW,
    );
    let agent = v
        .verify(
            &ctx,
            body,
            &route(Operation::Commit, Purpose::Pay),
            "verify-1",
        )
        .await
        .unwrap();
    (agent, rec)
}

async fn deny(
    auth: &LocalGrantAuthorizer<ManualClock>,
    actor: &VerifiedAgent,
    co: &CheckoutView,
    ev: &SignedEnvelope,
    state: GrantState,
) -> String {
    let err = auth
        .authorize(actor, co, ev, &Ledger(state))
        .await
        .unwrap_err();
    assert_eq!(err.kind, RejectKind::AuthorityDenied, "{err}");
    err.detail
}

#[tokio::test]
async fn an_allow_decision_is_bound_to_the_checkout_and_the_verification() {
    let (a, rec) = verified(1, "agent-a").await;
    let auth = authorizer(Environment::Local, ManualClock::new(NOW));
    let co = checkout(2);
    let d = auth
        .authorize(&a, &co, &seal(&grant(&rec)), &Ledger(GrantState::default()))
        .await
        .unwrap();
    assert_eq!(d.authority, "local-test-grant");
    assert_eq!(d.grant_id, "grant-1");
    assert_eq!(d.principal, "principal:alice");
    assert_eq!(d.checkout_hash, co.hash());
    assert_eq!(d.verification_decision, "verify-1");
    assert_eq!(d.actor_key_thumbprint, rec.thumbprint());
    assert_ne!(co.hash(), checkout(1).hash());
}

#[tokio::test]
async fn t17_another_agent_cannot_use_the_grant() {
    let (_a, rec_a) = verified(1, "agent-a").await;
    let (b, _) = verified(2, "agent-b").await;
    let auth = authorizer(Environment::Local, ManualClock::new(NOW));
    let why = deny(
        &auth,
        &b,
        &checkout(1),
        &seal(&grant(&rec_a)),
        GrantState::default(),
    )
    .await;
    assert!(why.contains("another agent"));
}

#[tokio::test]
async fn t18_expired_or_revoked_grants_deny_commitment() {
    let (a, rec) = verified(1, "agent-a").await;
    let clock = ManualClock::new(NOW);
    let auth = authorizer(Environment::Local, clock.clone());
    let ev = seal(&grant(&rec));
    deny(
        &auth,
        &a,
        &checkout(1),
        &ev,
        GrantState {
            revoked: true,
            spent_minor: 0,
        },
    )
    .await;
    clock.set(NOW + 3_600);
    deny(&auth, &a, &checkout(1), &ev, GrantState::default()).await;
    let err = authorizer(Environment::Local, ManualClock::new(NOW))
        .authorize(&a, &checkout(1), &ev, &DownLedger)
        .await
        .unwrap_err();
    assert_eq!(err.kind, RejectKind::Unavailable);
}

#[tokio::test]
async fn t19_merchant_item_total_and_currency_must_match_the_grant() {
    let (a, rec) = verified(1, "agent-a").await;
    let auth = authorizer(Environment::Local, ManualClock::new(NOW));
    let ev = seal(&grant(&rec));
    let none = GrantState::default();
    let mut co = checkout(1);
    co.merchant = "https://other.example".into();
    deny(&auth, &a, &co, &ev, none).await;
    let mut co = checkout(1);
    co.lines[0].item_id = "demo-002".into();
    deny(&auth, &a, &co, &ev, none).await;
    deny(&auth, &a, &checkout(3), &ev, none).await;
    let mut co = checkout(1);
    co.currency = "EUR".into();
    deny(&auth, &a, &co, &ev, none).await;
    // Over the remaining limit, given what was already spent.
    deny(
        &auth,
        &a,
        &checkout(2),
        &ev,
        GrantState {
            revoked: false,
            spent_minor: 3_000,
        },
    )
    .await;
    let mut free = checkout(1);
    free.total_minor = 0;
    deny(&auth, &a, &free, &ev, none).await;
}

#[tokio::test]
async fn forged_or_mislabelled_evidence_is_denied() {
    let (a, rec) = verified(1, "agent-a").await;
    let auth = authorizer(Environment::Local, ManualClock::new(NOW));
    let none = GrantState::default();
    let g = grant(&rec);
    // Signed by a key that is not a configured issuer.
    let rogue = arkavo_tap::envelope::seal(
        LOCAL_TEST_GRANT,
        ISSUER,
        &SigningKey::from_bytes(&[9; 32]),
        &serde_json::to_vec(&g).unwrap(),
    );
    deny(&auth, &a, &checkout(1), &rogue, none).await;
    let unknown_issuer = arkavo_tap::envelope::seal(
        LOCAL_TEST_GRANT,
        "nobody",
        &issuer_key(),
        &serde_json::to_vec(&g).unwrap(),
    );
    deny(&auth, &a, &checkout(1), &unknown_issuer, none).await;
    // A receipt is not a grant.
    let receipt = arkavo_tap::envelope::seal(
        "simulated-receipt",
        ISSUER,
        &issuer_key(),
        &serde_json::to_vec(&g).unwrap(),
    );
    deny(&auth, &a, &checkout(1), &receipt, none).await;
    // Mislabelled, wrong environment, wrong issuer name, not a grant at all.
    for mutate in [
        |g: &mut LocalTestGrant| g.label = "ap2-mandate".into(),
        |g: &mut LocalTestGrant| g.environment = "production".into(),
        |g: &mut LocalTestGrant| g.issuer = "someone-else".into(),
    ] {
        let mut bad = g.clone();
        mutate(&mut bad);
        deny(&auth, &a, &checkout(1), &seal(&bad), none).await;
    }
    let junk = arkavo_tap::envelope::seal(LOCAL_TEST_GRANT, ISSUER, &issuer_key(), b"not json");
    deny(&auth, &a, &checkout(1), &junk, none).await;
}

#[tokio::test]
async fn t24_development_grants_are_refused_outside_local() {
    let (a, rec) = verified(1, "agent-a").await;
    for env in [Environment::Staging, Environment::Production] {
        let auth = authorizer(env, ManualClock::new(NOW));
        deny(
            &auth,
            &a,
            &checkout(1),
            &seal(&grant(&rec)),
            GrantState::default(),
        )
        .await;
    }
}
