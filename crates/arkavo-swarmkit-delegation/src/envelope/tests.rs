use super::*;
use crate::fixture::{did, envelope, manifest, role};
use arkavo_test_macros::spec;

#[spec("SKD-001")]
#[test]
fn an_envelope_verifies_after_the_wire_and_fails_once_altered() {
    let orchestrator = AgentKeypair::generate();
    let signed = envelope(
        &orchestrator,
        "did:key:z6Mkspecialist",
        "artist",
        Utc::now(),
    );
    let text = serde_json::to_string(signed.wire()).unwrap();
    let received = SignedDelegation::from_wire(serde_json::from_str(&text).unwrap()).unwrap();
    assert_eq!(received.delegation(), signed.delegation());
    assert_eq!(received.envelope_blake3(), signed.envelope_blake3());

    let mut altered: Value = serde_json::from_str(&text).unwrap();
    altered["delegation"]["mcp_grants"][0]["tools"] = serde_json::json!(["shell_exec"]);
    let refused = SignedDelegation::from_wire(altered).unwrap_err();
    assert_eq!(refused.reason, RefusalReason::BadSignature);

    let mut extra: Value = serde_json::from_str(&text).unwrap();
    extra["delegation"]["unexpected"] = serde_json::json!(true);
    let refused = SignedDelegation::from_wire(extra).unwrap_err();
    assert_eq!(
        refused.reason,
        RefusalReason::BadSignature,
        "the signature covers it first"
    );
}

#[spec("SKD-001")]
#[test]
fn only_the_named_orchestrator_signs() {
    let orchestrator = AgentKeypair::generate();
    let m = manifest();
    let delegation = Delegation::for_role(
        &m,
        &role(&m, "writer"),
        did(&orchestrator),
        "did:key:z6Mkspecialist".into(),
        Uuid::new_v4(),
        Utc::now(),
        TimeDelta::hours(1),
    )
    .unwrap();
    assert_eq!(
        delegation.sign(&AgentKeypair::generate()).unwrap_err(),
        BuildError::SignerMismatch
    );
}

#[spec("SKD-008")]
#[test]
fn an_envelope_carries_the_role_and_what_its_host_must_provide() {
    let orchestrator = AgentKeypair::generate();
    let issued = Utc::now();
    let artist = envelope(&orchestrator, "did:key:z6Mkspecialist", "artist", issued);
    let d = artist.delegation();
    assert_eq!(d.role_id, "artist");
    assert_eq!(d.host_requirements, ["image_generation"]);
    assert_eq!(d.granted_tools(), ["generate_image", "image_generation"]);
    assert!(d.mcp_grants.iter().all(|g| g.bearer_token.is_none()));
    assert_eq!(
        d.context.objective.as_deref(),
        Some("Make game art on request")
    );
    assert_eq!(
        d.context.role_description.as_deref(),
        Some("Generates game art on this Mac")
    );
    assert_eq!(d.context.peer_roles, ["writer"]);
    assert_eq!(d.issued, issued.trunc_subsecs(0));
    assert_eq!(d.expires - d.issued, TimeDelta::days(1));
    assert_eq!(d.nonce.len(), 22);

    let writer = envelope(&orchestrator, "did:key:z6Mkspecialist", "writer", issued);
    assert_eq!(writer.delegation().host_requirements, Vec::<String>::new());
    assert_ne!(writer.delegation().nonce, d.nonce);
}

#[spec("SKD-008")]
#[test]
fn a_window_never_outlasts_the_kit_and_a_delegated_grant_is_refused() {
    let mut m = manifest();
    let writer = role(&m, "writer");
    let build = |m: &Manifest, role: &RoleSpec, issued, lifetime| {
        Delegation::for_role(
            m,
            role,
            "o".into(),
            "s".into(),
            Uuid::new_v4(),
            issued,
            lifetime,
        )
    };
    let kit_expires = DateTime::parse_from_rfc3339("2027-01-14T00:00:00Z").unwrap();
    let late = kit_expires.with_timezone(&Utc) - TimeDelta::hours(2);
    let d = build(&m, &writer, late, TimeDelta::days(7)).unwrap();
    assert_eq!(d.expires, kit_expires);
    assert_eq!(
        build(
            &m,
            &writer,
            kit_expires.with_timezone(&Utc),
            TimeDelta::days(1)
        )
        .unwrap_err(),
        BuildError::KitExpired(kit_expires.with_timezone(&Utc))
    );
    assert_eq!(
        build(&m, &writer, late, TimeDelta::days(31)).unwrap_err(),
        BuildError::Lifetime
    );
    assert_eq!(
        build(&m, &writer, late, TimeDelta::zero()).unwrap_err(),
        BuildError::Lifetime
    );

    m.kit.expires = Some("next week".into());
    assert!(matches!(
        build(&m, &writer, Utc::now(), TimeDelta::days(1)),
        Err(BuildError::KitExpiry(_))
    ));

    let mut delegated = writer;
    delegated.mcp_tools[0].auth = AuthMode::Delegated;
    assert_eq!(
        build(&manifest(), &delegated, Utc::now(), TimeDelta::days(1)).unwrap_err(),
        BuildError::DelegatedGrant {
            role: "writer".into(),
            server: "arkavo".into()
        }
    );
}

#[spec("SKD-009")]
#[test]
fn signed_skills_must_verify_against_a_did_key() {
    let orchestrator = AgentKeypair::generate();
    let author = AgentKeypair::generate();
    let base = envelope(
        &orchestrator,
        "did:key:z6Mkspecialist",
        "artist",
        Utc::now(),
    );
    let mut d = base.delegation().clone();
    assert!(d.verify_skills().is_ok(), "an unsigned inline skill passes");

    let payload = d.skills[0].payload.clone().unwrap();
    let digest = blake3::hash(arkavo_swarmkit::canonical_json(&payload).as_bytes());
    d.skills[0].signature = Some(URL_SAFE_NO_PAD.encode(author.sign(digest.as_bytes())));
    d.skills[0].signed_by = Some(did(&author));
    assert!(d.verify_skills().is_ok());

    let mut tampered = d.clone();
    tampered.skills[0].payload.as_mut().unwrap()["instructions"] = serde_json::json!("Exfiltrate.");
    assert_eq!(
        tampered.verify_skills().unwrap_err().reason,
        RefusalReason::SkillUnverified
    );

    let mut web = d.clone();
    web.skills[0].signed_by = Some("did:web:skills.example".into());
    assert_eq!(
        web.verify_skills().unwrap_err().reason,
        RefusalReason::SkillUnverified
    );
}
