use super::*;
use crate::types::{AgentCapabilities, AgentExtension, AgentSkill};
use arkavo_crypto::P256SigningKeypair;
use arkavo_test_macros::spec;

fn card() -> AgentCard {
    AgentCard {
        name: "planner".to_string(),
        description: Some("Plans the week".to_string()),
        url: "http://10.0.0.140:65494".to_string(),
        provider: None,
        version: "0.107.0".to_string(),
        protocol_versions: vec!["0.3".to_string()],
        default_input_modes: vec!["text/plain".to_string()],
        default_output_modes: vec!["text/plain".to_string()],
        capabilities: AgentCapabilities::default(),
        skills: vec![AgentSkill {
            id: "plan".to_string(),
            name: "plan".to_string(),
            description: Some("Écrit le plan — naïvely".to_string()),
            tags: vec!["mcp".to_string()],
            examples: vec![],
            input_modes: vec![],
            output_modes: vec![],
        }],
        security_schemes: vec![],
        security: vec![],
        extensions: vec![],
        signatures: vec![],
    }
}

fn key_id(keypair: &P256SigningKeypair) -> String {
    let did = keypair.public_key().to_did_key();
    let multibase = did.trim_start_matches("did:key:").to_string();
    format!("{did}#{multibase}")
}

fn signed(keypair: &P256SigningKeypair) -> Value {
    let mut card = card();
    sign(&mut card, &key_id(keypair), |input| {
        Ok::<_, String>(keypair.sign_p1363(input))
    })
    .unwrap();
    serde_json::to_value(card).unwrap()
}

fn header(card: &Value) -> Value {
    let protected = card["signatures"][0]["protected"].as_str().unwrap();
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(protected).unwrap()).unwrap()
}

#[spec("SRV-017")]
#[test]
fn a_signed_card_names_and_proves_its_signer() {
    let keypair = P256SigningKeypair::generate();
    let card = signed(&keypair);
    let protected = header(&card);
    assert_eq!(protected["alg"], "ES256");
    assert_eq!(protected["typ"], "JOSE");
    assert_eq!(protected["kid"], key_id(&keypair));
    assert_eq!(verify(&card), Ok(keypair.public_key().to_did_key()));
}

#[spec("SRV-017")]
#[test]
fn whitespace_and_member_order_do_not_matter() {
    let keypair = P256SigningKeypair::generate();
    let card = signed(&keypair);
    // Received as another serializer might write it: pretty, members in
    // reverse order.
    let mut fields: Vec<_> = card.as_object().unwrap().clone().into_iter().collect();
    fields.reverse();
    let text = format!(
        "{{\n{}\n}}",
        fields
            .iter()
            .map(|(k, v)| format!(
                "  {}: {}",
                Value::String(k.clone()),
                serde_json::to_string_pretty(v).unwrap()
            ))
            .collect::<Vec<_>>()
            .join(",\n")
    );
    let received: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(verify(&received), Ok(keypair.public_key().to_did_key()));
}

#[spec("SRV-017")]
#[test]
fn a_changed_card_does_not_verify() {
    let keypair = P256SigningKeypair::generate();
    let mut card = signed(&keypair);
    card["name"] = Value::String("impostor".to_string());
    assert!(matches!(verify(&card), Err(CardSignatureError::Invalid(_))));
}

#[spec("SRV-017")]
#[test]
fn a_signature_by_a_key_other_than_the_kid_does_not_verify() {
    let named = P256SigningKeypair::generate();
    let actual = P256SigningKeypair::generate();
    let mut card = card();
    sign(&mut card, &key_id(&named), |input| {
        Ok::<_, String>(actual.sign_p1363(input))
    })
    .unwrap();
    let card = serde_json::to_value(card).unwrap();
    assert!(matches!(verify(&card), Err(CardSignatureError::Invalid(_))));
}

#[spec("SRV-017")]
#[test]
fn a_kid_whose_fragment_is_another_key_is_refused() {
    let keypair = P256SigningKeypair::generate();
    let other = P256SigningKeypair::generate().public_key().to_did_key();
    let did = keypair.public_key().to_did_key();
    let kid = format!("{did}#{}", other.trim_start_matches("did:key:"));
    let mut card = card();
    sign(&mut card, &kid, |input| {
        Ok::<_, String>(keypair.sign_p1363(input))
    })
    .unwrap();
    let card = serde_json::to_value(card).unwrap();
    assert!(
        matches!(verify(&card), Err(CardSignatureError::Invalid(why)) if why.contains("does not have"))
    );
}

#[spec("SRV-017")]
#[test]
fn only_es256_is_accepted() {
    let keypair = P256SigningKeypair::generate();
    let mut card = signed(&keypair);
    let mut protected = header(&card);
    protected["alg"] = Value::String("EdDSA".to_string());
    card["signatures"][0]["protected"] =
        Value::String(URL_SAFE_NO_PAD.encode(protected.to_string()));
    assert!(
        matches!(verify(&card), Err(CardSignatureError::Invalid(why)) if why.contains("ES256"))
    );
}

#[spec("SRV-017")]
#[test]
fn an_unsigned_card_says_so() {
    let card = serde_json::to_value(card()).unwrap();
    assert!(
        card.get("signatures").is_none(),
        "an unsigned card has no signatures member"
    );
    assert_eq!(verify(&card), Err(CardSignatureError::Unsigned));
}

#[test]
fn a_failing_signer_leaves_the_card_unsigned() {
    let mut card = card();
    let result = sign(&mut card, "did:key:zDnX#zDnX", |_| Err("enclave refused"));
    assert_eq!(
        result,
        Err(CardSignatureError::Signer("enclave refused".to_string()))
    );
    assert!(card.signatures.is_empty());
}

#[test]
fn a_card_with_a_fraction_is_not_signed() {
    let mut card = card();
    card.extensions.push(AgentExtension {
        uri: "https://arkavo.com/ext/load".to_string(),
        data: Some(serde_json::json!({ "load": 0.5 })),
    });
    let result = sign(&mut card, "did:key:zDnX#zDnX", |_| {
        Ok::<_, String>([0u8; 64])
    });
    assert!(matches!(result, Err(CardSignatureError::Canonical(_))));
}

/// RFC 8785 §3.2.3: members sort by UTF-16 code units, which puts an emoji
/// (a surrogate pair) before U+FB33, unlike code-point or UTF-8 order.
#[test]
fn members_sort_as_rfc_8785_requires() {
    let input: Value = serde_json::from_str(
        r#"{"€":"Euro Sign","\r":"Carriage Return","דּ":"Hebrew Letter Dalet With Dagesh","1":"One","😀":"Emoji: Grinning Face","\u0080":"Control","ö":"Latin Small Letter O With Diaeresis"}"#,
    )
    .unwrap();
    let expected = "{\"\\r\":\"Carriage Return\",\"1\":\"One\",\"\u{80}\":\"Control\",\
        \"\u{f6}\":\"Latin Small Letter O With Diaeresis\",\"\u{20ac}\":\"Euro Sign\",\
        \"\u{1f600}\":\"Emoji: Grinning Face\",\"\u{fb33}\":\"Hebrew Letter Dalet With Dagesh\"}";
    assert_eq!(canonical(&input).unwrap(), expected);
}

/// RFC 8785 §3.2.2.2's string example.
#[test]
fn strings_escape_as_rfc_8785_requires() {
    // The example's input string, decoded: a control character and
    // escaped quotes and backslashes among plain text.
    let input = Value::String("\u{20ac}$\u{f}\nA'B\"\\\\\"/".to_string());
    let expected = format!("\"\u{20ac}${}u000f\\nA'B\\\"\\\\\\\\\\\"/\"", '\\');
    assert_eq!(canonical(&input).unwrap(), expected);
}

/// A2A v1.0 §8.4.1's example, after its default removal.
#[test]
fn the_a2a_example_canonicalizes_as_published() {
    let input: Value = serde_json::from_str(
        r#"{ "name": "Example Agent", "description": "", "capabilities": { "streaming": false, "pushNotifications": false }, "skills": [] }"#,
    )
    .unwrap();
    assert_eq!(
        canonical(&input).unwrap(),
        r#"{"capabilities":{"pushNotifications":false,"streaming":false},"description":"","name":"Example Agent","skills":[]}"#
    );
}

#[test]
fn integers_beyond_double_precision_are_refused() {
    assert_eq!(
        canonical(&serde_json::json!(9_007_199_254_740_992_u64)).unwrap(),
        "9007199254740992"
    );
    assert!(canonical(&serde_json::json!(9_007_199_254_740_993_u64)).is_err());
    assert_eq!(canonical(&serde_json::json!(-42)).unwrap(), "-42");
}
