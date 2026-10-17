use super::*;
use crate::fixture::{did, envelope};
use crate::revocation::Revocation;
use arkavo_crypto::AgentKeypair;
use arkavo_test_macros::spec;
use uuid::Uuid;

struct Fixture {
    _dir: tempfile::TempDir,
    store: StateStore,
    me: String,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::new(dir.path());
    Fixture {
        _dir: dir,
        store,
        me: did(&AgentKeypair::generate()),
    }
}

const FIRST_USE: Policy = Policy { first_use: true };

fn open(f: &Fixture, policy: Policy) -> Specialist {
    Specialist::open(f.me.clone(), f.store.clone(), policy, Utc::now()).unwrap()
}

fn accept(s: &mut Specialist, signed: &SignedDelegation) -> Result<Ready, Refusal> {
    let admitted = s.admit(signed.wire().clone(), Utc::now())?;
    Ok(s.commit(admitted, Utc::now()).unwrap())
}

#[spec("SKD-002")]
#[test]
fn an_envelope_is_for_one_specialist_one_window_and_one_use() {
    let f = fixture();
    let mut s = open(&f, FIRST_USE);
    let orchestrator = AgentKeypair::generate();
    let now = Utc::now();
    let reason = |s: &Specialist, signed: SignedDelegation| {
        s.admit(signed.wire().clone(), now).unwrap_err().reason
    };

    assert_eq!(
        reason(
            &s,
            envelope(&orchestrator, "did:key:z6Mkother", "writer", now)
        ),
        RefusalReason::WrongSpecialist
    );
    assert_eq!(
        reason(
            &s,
            envelope(&orchestrator, &f.me, "writer", now - TimeDelta::days(2))
        ),
        RefusalReason::Expired
    );
    assert_eq!(
        reason(
            &s,
            envelope(&orchestrator, &f.me, "writer", now + TimeDelta::hours(1))
        ),
        RefusalReason::NotYetValid
    );

    let signed = envelope(&orchestrator, &f.me, "writer", now);
    let ready = accept(&mut s, &signed).unwrap();
    assert_eq!(ready.envelope_blake3, signed.envelope_blake3());
    assert_eq!(ready.specialist, f.me);
    assert_eq!(reason(&s, signed), RefusalReason::Replayed);
}

#[spec("SKD-003")]
#[test]
fn the_first_orchestrator_whose_role_is_accepted_is_the_only_one() {
    let f = fixture();
    let mut s = open(&f, FIRST_USE);
    assert_eq!(s.status(Vec::new(), 0.0).binds_by, [BindingMode::FirstUse]);
    let (first, later) = (AgentKeypair::generate(), AgentKeypair::generate());
    let now = Utc::now();

    // A role this host refuses at step 6 is never committed, so it binds nothing.
    let refused = s
        .admit(envelope(&later, &f.me, "artist", now).wire().clone(), now)
        .unwrap();
    assert!(refused.binds());
    drop(refused);
    assert!(s.binding().is_none());

    accept(&mut s, &envelope(&first, &f.me, "writer", now)).unwrap();
    let binding = s.binding().unwrap();
    assert_eq!(
        (binding.orchestrator.as_str(), binding.mode),
        (did(&first).as_str(), BindingMode::FirstUse)
    );
    assert_eq!(s.status(Vec::new(), 0.0).binds_by, []);

    let refused = accept(&mut s, &envelope(&later, &f.me, "artist", now)).unwrap_err();
    assert_eq!(refused.reason, RefusalReason::NotBoundOrchestrator);
    accept(&mut s, &envelope(&first, &f.me, "artist", now)).unwrap();
    assert_eq!(
        s.active().unwrap().role_id,
        "artist",
        "the bound orchestrator replaces the role"
    );

    let closed = fixture();
    let s = open(&closed, Policy { first_use: false });
    assert_eq!(s.status(Vec::new(), 0.0).binds_by, []);
    let refused = s
        .admit(
            envelope(&first, &closed.me, "writer", now).wire().clone(),
            now,
        )
        .unwrap_err();
    assert_eq!(refused.reason, RefusalReason::BindingRequired);
}

#[spec("SKD-006")]
#[test]
fn a_role_survives_a_restart_until_it_expires_and_cannot_be_rolled_back() {
    let f = fixture();
    let orchestrator = AgentKeypair::generate();
    let now = Utc::now();
    let older = envelope(&orchestrator, &f.me, "artist", now - TimeDelta::minutes(30));
    let newer = envelope(&orchestrator, &f.me, "writer", now);
    {
        let mut s = open(&f, FIRST_USE);
        accept(&mut s, &older).unwrap();
        accept(&mut s, &newer).unwrap();
    }
    let mut s = open(&f, FIRST_USE);
    assert_eq!(s.active().unwrap().role_id, "writer");
    assert_eq!(s.binding().unwrap().orchestrator, did(&orchestrator));
    let summary = s.status(Vec::new(), 0.0).delegation.unwrap();
    assert_eq!(summary.role_id, "writer");

    // The nonce cache starts empty after a restart; the issue time still
    // keeps the older role out.
    let refused = s.admit(older.wire().clone(), now).unwrap_err();
    assert_eq!(refused.reason, RefusalReason::Replayed);

    assert!(!s.expire(now).unwrap());
    assert!(s.expire(now + TimeDelta::days(2)).unwrap());
    assert!(s.active().is_none());
    let s = Specialist::open(f.me, f.store, FIRST_USE, now).unwrap();
    assert!(s.active().is_none(), "expiry is persisted");
    assert!(s.binding().is_some(), "the binding outlives the role");
}

#[spec("SKD-006")]
#[test]
fn a_stored_role_from_another_orchestrator_or_specialist_is_dropped() {
    let f = fixture();
    let orchestrator = AgentKeypair::generate();
    let now = Utc::now();
    let foreign = envelope(&orchestrator, "did:key:z6Mkother", "writer", now);
    f.store.save_delegation(Some(foreign.wire())).unwrap();
    assert!(open(&f, FIRST_USE).active().is_none());
    assert!(f.store.load_delegation().unwrap().is_none());

    let unbound = envelope(&orchestrator, &f.me, "writer", now);
    f.store.save_delegation(Some(unbound.wire())).unwrap();
    assert!(
        open(&f, FIRST_USE).active().is_none(),
        "no binding vouches for it"
    );
}

#[spec("SKD-007")]
#[test]
fn the_operator_names_the_orchestrator_and_can_release_it() {
    let f = fixture();
    let mut s = open(&f, FIRST_USE);
    let (named, other) = (AgentKeypair::generate(), AgentKeypair::generate());
    let now = Utc::now();
    accept(&mut s, &envelope(&other, &f.me, "writer", now)).unwrap();

    s.bind_operator(&did(&named), now).unwrap();
    assert_eq!(s.binding().unwrap().mode, BindingMode::Operator);
    assert!(
        s.active().is_none(),
        "the old orchestrator's role goes with its binding"
    );
    let refused = accept(&mut s, &envelope(&other, &f.me, "writer", now)).unwrap_err();
    assert_eq!(refused.reason, RefusalReason::NotBoundOrchestrator);
    accept(&mut s, &envelope(&named, &f.me, "writer", now)).unwrap();
    assert!(matches!(
        s.bind_operator("did:web:example.com", now),
        Err(StoreError::NotADidKey(_))
    ));

    s.release().unwrap();
    let s = open(&f, FIRST_USE);
    assert!(s.binding().is_none() && s.active().is_none());
}

#[spec("SKD-005")]
#[test]
fn only_the_bound_orchestrator_revokes_once() {
    let f = fixture();
    let mut s = open(&f, FIRST_USE);
    let (bound, other) = (AgentKeypair::generate(), AgentKeypair::generate());
    let now = Utc::now();
    let flight = accept(&mut s, &envelope(&bound, &f.me, "writer", now))
        .unwrap()
        .flight_id;
    let revoke = |by: &AgentKeypair, flight_id, release, issued| {
        Revocation::new(did(by), f.me.clone(), flight_id, release, issued)
            .sign(by)
            .unwrap()
            .wire()
            .clone()
    };
    let refused = |r: Result<(Revoked, bool), RevokeError>| match r.unwrap_err() {
        RevokeError::Refused(refusal) => refusal.reason,
        RevokeError::Store(e) => panic!("{e}"),
    };

    assert_eq!(
        refused(s.revoke(revoke(&other, None, false, now), now)),
        RefusalReason::NotBoundOrchestrator
    );
    assert_eq!(
        refused(s.revoke(
            revoke(&bound, None, false, now - TimeDelta::minutes(6)),
            now
        )),
        RefusalReason::Expired
    );
    let (_, dropped) = s
        .revoke(revoke(&bound, Some(Uuid::new_v4()), false, now), now)
        .unwrap();
    assert!(!dropped, "another flight's revocation leaves this one");

    let wire = revoke(&bound, Some(flight), false, now);
    let (revoked, dropped) = s.revoke(wire.clone(), now).unwrap();
    assert!(dropped && s.active().is_none());
    assert_eq!(revoked.specialist, f.me);
    assert_eq!(refused(s.revoke(wire, now)), RefusalReason::Replayed);
    assert!(s.binding().is_some());

    let (_, released) = s.revoke(revoke(&bound, None, true, now), now).unwrap();
    assert!(released && s.binding().is_none());
    assert!(open(&f, FIRST_USE).binding().is_none());
}

#[test]
fn an_oversized_envelope_is_refused_before_parsing() {
    let f = fixture();
    let s = open(&f, FIRST_USE);
    let huge = serde_json::json!({"delegation": {"padding": "x".repeat(MAX_ENVELOPE_BYTES)}});
    assert_eq!(
        s.admit(huge, Utc::now()).unwrap_err().reason,
        RefusalReason::InvalidEnvelope
    );
}
