use super::*;
use arkavo_crypto::{AgentKeypair, P256SigningKeypair};
use arkavo_test_macros::spec;

const NOW: u64 = 1_800_000_000;

struct Workload {
    keypair: P256SigningKeypair,
    did: String,
}

impl Workload {
    fn new() -> Self {
        let keypair = P256SigningKeypair::generate();
        let did = keypair.public_key().to_did_key();
        Self { keypair, did }
    }

    fn key_id(&self) -> String {
        format!("{}#{}", self.did, &self.did["did:key:".len()..])
    }

    fn statement(&self, purpose: Purpose, key: String, generation: u64) -> KeyStatement {
        KeyStatement {
            issuer: self.did.clone(),
            key,
            purpose,
            generation,
            not_before: NOW - 60,
            not_after: NOW + 3600,
        }
    }

    fn issue(&self, statement: &KeyStatement) -> String {
        issue(statement, &self.key_id(), |input| {
            Ok::<_, String>(self.keypair.sign_p1363(input))
        })
        .unwrap()
    }

    /// A JWS with any header and payload, for what `issue` refuses to make.
    fn sign_raw(&self, header: &Value, payload: &Value) -> String {
        let input = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header.to_string()),
            URL_SAFE_NO_PAD.encode(payload.to_string())
        );
        let signature = self.keypair.sign_p1363(input.as_bytes());
        format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature))
    }

    fn header(&self) -> Value {
        serde_json::json!({ "alg": "ES256", "typ": MEDIA_TYPE, "kid": self.key_id() })
    }
}

fn gossip_key() -> String {
    AgentKeypair::generate().public_key().to_did_key()
}

fn tdf_key() -> String {
    P256SigningKeypair::generate().public_key().to_did_key()
}

fn payload(statement: &KeyStatement) -> Value {
    serde_json::to_value(statement).unwrap()
}

#[spec("INGRESS-006")]
#[test]
fn a_statement_names_the_agent_and_its_key() {
    let workload = Workload::new();
    let gossip = AgentKeypair::generate().public_key();
    let made = workload.statement(Purpose::Gossip, gossip.to_did_key(), 3);

    let read = verify(&workload.issue(&made), NOW).unwrap();
    assert_eq!(read, made);
    assert_eq!(
        read.ed25519_key().unwrap().to_did_key(),
        gossip.to_did_key()
    );
    assert!(read.p256_key().is_err());
}

#[spec("INGRESS-006")]
#[test]
fn a_tdf_statement_names_a_p256_key() {
    let workload = Workload::new();
    let tdf = P256SigningKeypair::generate().public_key();
    let made = workload.statement(Purpose::Tdf, tdf.to_did_key(), 0);
    let read = verify(&workload.issue(&made), NOW).unwrap();
    assert_eq!(read.p256_key().unwrap().to_did_key(), tdf.to_did_key());
}

#[spec("INGRESS-006")]
#[test]
fn the_payload_is_json_with_registered_claim_names() {
    let workload = Workload::new();
    let compact = workload.issue(&workload.statement(Purpose::Iroh, gossip_key(), 1));
    let parts: Vec<&str> = compact.split('.').collect();
    let header: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
    assert_eq!(header["typ"], MEDIA_TYPE);
    assert_eq!(header["kid"], workload.key_id());
    for name in ["iss", "sub", "purpose", "gen", "nbf", "exp"] {
        assert!(claims.get(name).is_some(), "{name} in {claims}");
    }
    assert_eq!(claims["purpose"], "iroh");
}

#[spec("INGRESS-006")]
#[test]
fn a_changed_statement_does_not_verify() {
    let workload = Workload::new();
    let compact = workload.issue(&workload.statement(Purpose::Gossip, gossip_key(), 1));
    let parts: Vec<&str> = compact.split('.').collect();
    let other = payload(&workload.statement(Purpose::Gossip, gossip_key(), 1));
    let swapped = format!(
        "{}.{}.{}",
        parts[0],
        URL_SAFE_NO_PAD.encode(other.to_string()),
        parts[2]
    );
    assert!(matches!(
        verify(&swapped, NOW),
        Err(StatementError::Signature(_))
    ));
}

#[spec("INGRESS-006")]
#[test]
fn a_key_id_naming_another_workload_does_not_verify() {
    let workload = Workload::new();
    let impostor = Workload::new();
    let statement = workload.statement(Purpose::Gossip, gossip_key(), 1);
    // The impostor signs, but names the real workload's key.
    let compact = impostor.sign_raw(&workload.header(), &payload(&statement));
    assert!(matches!(
        verify(&compact, NOW),
        Err(StatementError::Signature(_))
    ));
}

#[spec("INGRESS-006")]
#[test]
fn a_statement_in_another_agents_name_is_refused() {
    let workload = Workload::new();
    let victim = Workload::new();
    let claims = payload(&victim.statement(Purpose::Gossip, gossip_key(), 1));
    let compact = workload.sign_raw(&workload.header(), &claims);
    let err = verify(&compact, NOW).unwrap_err();
    assert!(err.to_string().contains("in the name of"), "{err}");

    let mismatched = victim.statement(Purpose::Gossip, gossip_key(), 1);
    assert!(
        issue(&mismatched, &workload.key_id(), |input| {
            Ok::<_, String>(workload.keypair.sign_p1363(input))
        })
        .is_err()
    );
}

#[spec("INGRESS-006")]
#[test]
fn a_statement_holds_only_inside_its_window() {
    let workload = Workload::new();
    let statement = workload.statement(Purpose::Gossip, gossip_key(), 1);
    let compact = workload.issue(&statement);
    assert!(verify(&compact, statement.not_before).is_ok());
    assert!(verify(&compact, statement.not_after - 1).is_ok());
    for now in [statement.not_before - 1, statement.not_after] {
        assert!(matches!(
            verify(&compact, now),
            Err(StatementError::OutsideWindow { .. })
        ));
    }
}

#[spec("INGRESS-006")]
#[test]
fn an_empty_window_is_refused() {
    let workload = Workload::new();
    let mut statement = workload.statement(Purpose::Gossip, gossip_key(), 1);
    statement.not_after = statement.not_before;
    assert!(
        issue(&statement, &workload.key_id(), |input| {
            Ok::<_, String>(workload.keypair.sign_p1363(input))
        })
        .is_err()
    );
}

#[spec("INGRESS-006")]
#[test]
fn a_key_of_the_wrong_kind_for_its_purpose_is_refused() {
    let workload = Workload::new();
    for (purpose, key) in [
        (Purpose::Gossip, tdf_key()),
        (Purpose::Iroh, tdf_key()),
        (Purpose::Tdf, gossip_key()),
    ] {
        let claims = payload(&workload.statement(purpose, key, 1));
        let compact = workload.sign_raw(&workload.header(), &claims);
        assert!(
            matches!(verify(&compact, NOW), Err(StatementError::Key(_))),
            "{purpose:?}"
        );
    }
}

#[spec("INGRESS-006")]
#[test]
fn a_key_named_other_than_by_its_canonical_did_is_refused() {
    let workload = Workload::new();
    let tdf = P256SigningKeypair::generate().public_key();
    // The same key, uncompressed: a second name for one key.
    let mut prefixed = vec![0x80, 0x24];
    prefixed.extend_from_slice(&tdf.to_sec1_bytes());
    let uncompressed = format!("did:key:z{}", bs58::encode(&prefixed).into_string());
    assert!(P256VerifyingKey::from_did_key(&uncompressed).is_ok());

    let claims = payload(&workload.statement(Purpose::Tdf, uncompressed, 1));
    let compact = workload.sign_raw(&workload.header(), &claims);
    assert!(matches!(verify(&compact, NOW), Err(StatementError::Key(_))));
}

#[spec("INGRESS-006")]
#[test]
fn the_workload_key_is_never_its_own_subordinate() {
    let workload = Workload::new();
    let claims = payload(&workload.statement(Purpose::Tdf, workload.did.clone(), 1));
    let compact = workload.sign_raw(&workload.header(), &claims);
    assert!(matches!(verify(&compact, NOW), Err(StatementError::Key(_))));
}

#[spec("INGRESS-006")]
#[test]
fn another_token_the_workload_key_signs_is_not_a_statement() {
    let workload = Workload::new();
    let claims = payload(&workload.statement(Purpose::Gossip, gossip_key(), 1));
    let headers = [
        serde_json::json!({ "alg": "ES256", "typ": "JOSE", "kid": workload.key_id() }),
        serde_json::json!({ "alg": "ES256", "kid": workload.key_id() }),
        serde_json::json!({ "alg": "none", "typ": MEDIA_TYPE, "kid": workload.key_id() }),
        serde_json::json!({ "alg": "HS256", "typ": MEDIA_TYPE, "kid": workload.key_id() }),
        serde_json::json!({
            "alg": "ES256", "typ": MEDIA_TYPE, "kid": workload.key_id(), "crit": ["b64"]
        }),
    ];
    for header in headers {
        let compact = workload.sign_raw(&header, &claims);
        assert!(
            matches!(verify(&compact, NOW), Err(StatementError::Malformed(_))),
            "{header}"
        );
    }
}

#[spec("INGRESS-006")]
#[test]
fn text_that_is_not_a_compact_jws_is_refused() {
    for text in ["", "a.b", "a.b.c.d", "!!.??.**"] {
        assert!(
            matches!(verify(text, NOW), Err(StatementError::Malformed(_))),
            "{text}"
        );
    }
}

#[spec("INGRESS-006")]
#[test]
fn a_signer_that_fails_is_reported() {
    let workload = Workload::new();
    let statement = workload.statement(Purpose::Gossip, gossip_key(), 1);
    let err = issue(&statement, &workload.key_id(), |_| {
        Err::<[u8; 64], _>("enclave locked")
    })
    .unwrap_err();
    assert_eq!(err, StatementError::Signer("enclave locked".to_string()));
}

#[spec("INGRESS-006")]
#[test]
fn an_older_generation_is_refused_once_a_newer_one_is_seen() {
    let workload = Workload::new();
    let mut seen = Generations::default();
    let first = workload.statement(Purpose::Gossip, gossip_key(), 1);
    let second = workload.statement(Purpose::Gossip, gossip_key(), 2);

    seen.admit(&first).unwrap();
    seen.admit(&first).unwrap();
    seen.admit(&second).unwrap();
    assert!(matches!(
        seen.admit(&first),
        Err(StatementError::Superseded {
            seen: 2,
            presented: 1,
            ..
        })
    ));
    seen.admit(&second).unwrap();
}

#[spec("INGRESS-006")]
#[test]
fn two_keys_for_one_generation_are_refused() {
    let workload = Workload::new();
    let mut seen = Generations::default();
    seen.admit(&workload.statement(Purpose::Gossip, gossip_key(), 4))
        .unwrap();
    assert!(matches!(
        seen.admit(&workload.statement(Purpose::Gossip, gossip_key(), 4)),
        Err(StatementError::Conflicting { generation: 4, .. })
    ));
}

#[spec("INGRESS-006")]
#[test]
fn generations_count_per_agent_and_purpose() {
    let planner = Workload::new();
    let coder = Workload::new();
    let mut seen = Generations::default();
    seen.admit(&planner.statement(Purpose::Gossip, gossip_key(), 9))
        .unwrap();
    seen.admit(&planner.statement(Purpose::Iroh, gossip_key(), 1))
        .unwrap();
    seen.admit(&coder.statement(Purpose::Gossip, gossip_key(), 1))
        .unwrap();
}
