//! `Signature-Input` and `Signature` (RFC 9421 section 4), parsed as RFC 8941 dictionaries.
//!
//! Strictness, beyond what RFC 9421 requires:
//! - each field holds exactly one signature, under the same label: two signatures are
//!   ambiguous about which one the merchant relied on;
//! - each field must already be in canonical serialization. Structured Fields let a later
//!   duplicate key or parameter silently replace an earlier one; a value that does not
//!   re-serialize to itself is refused instead of repaired;
//! - covered components are plain lower-case names without parameters, each named once;
//! - the parameters are exactly `created`, `expires`, `keyid`, `alg`, `nonce` and `tag`.

use crate::error::Rejection;
use sfv::{
    BareItem, Dictionary, FieldType, InnerList, Item, Key, ListEntry, Parameters, Parser, Version,
};

/// The signature parameters every profile requires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureParams {
    pub created: i64,
    pub expires: i64,
    pub keyid: String,
    pub alg: String,
    pub nonce: String,
    pub tag: String,
}

/// One parsed signature: what it covers, its parameters, and the bytes signed for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSignature {
    pub label: String,
    pub components: Vec<String>,
    pub params: SignatureParams,
    /// The serialized inner list and parameters: the `@signature-params` value.
    pub signature_params: String,
    pub signature: Vec<u8>,
}

const PARAM_NAMES: [&str; 6] = ["created", "expires", "keyid", "alg", "nonce", "tag"];

/// Parses both fields of one signature. Profile rules are applied by the verifier.
pub fn parse(signature_input: &str, signature: &str) -> Result<ParsedSignature, Rejection> {
    let (label, entry) = single_member(signature_input, "Signature-Input")?;
    let (sig_label, sig_entry) = single_member(signature, "Signature")?;
    if label != sig_label {
        return Err(Rejection::malformed(format!(
            "Signature-Input label {label:?} has no matching Signature"
        )));
    }
    let ListEntry::InnerList(list) = entry else {
        return Err(Rejection::malformed(
            "Signature-Input member is not an inner list",
        ));
    };
    let ListEntry::Item(Item {
        bare_item: BareItem::ByteSequence(sig),
        params,
    }) = sig_entry
    else {
        return Err(Rejection::malformed(
            "Signature member is not a byte sequence",
        ));
    };
    if !params.is_empty() {
        return Err(Rejection::malformed("Signature member carries parameters"));
    }
    let components = components(&list)?;
    let params = params_of(&list.params)?;
    let signature_params = signature_input[label.len() + 1..].to_owned();
    Ok(ParsedSignature {
        label,
        components,
        params,
        signature_params,
        signature: sig,
    })
}

/// The canonical `Signature-Input` value for one signature, and its `@signature-params`.
///
/// The second value is the part after `label=`. Parameters go in the order RFC 9421's own
/// examples and common implementations use, since some verifiers re-serialize them in that
/// order instead of using the received text.
pub fn signature_input(
    label: &str,
    components: &[&str],
    params: &SignatureParams,
) -> Result<(String, String), Rejection> {
    let items = components
        .iter()
        .map(|c| Ok(Item::new(sf_string(c)?)))
        .collect::<Result<Vec<_>, Rejection>>()?;
    let mut list = InnerList::new(items);
    list.params
        .insert(key("created")?, BareItem::from(int(params.created)?));
    list.params
        .insert(key("expires")?, BareItem::from(int(params.expires)?));
    list.params.insert(key("nonce")?, sf_string(&params.nonce)?);
    list.params.insert(key("alg")?, sf_string(&params.alg)?);
    list.params.insert(key("keyid")?, sf_string(&params.keyid)?);
    list.params.insert(key("tag")?, sf_string(&params.tag)?);
    let mut dict = Dictionary::new();
    dict.insert(key(label)?, ListEntry::InnerList(list));
    let value = dict
        .serialize()
        .ok_or_else(|| Rejection::malformed("empty Signature-Input"))?;
    let signature_params = value[label.len() + 1..].to_owned();
    Ok((value, signature_params))
}

/// The canonical `Signature` value for one signature.
pub fn signature(label: &str, signature: &[u8]) -> Result<String, Rejection> {
    let mut dict = Dictionary::new();
    dict.insert(
        key(label)?,
        ListEntry::Item(Item::new(BareItem::from(signature.to_vec()))),
    );
    dict.serialize()
        .ok_or_else(|| Rejection::malformed("empty Signature"))
}

fn single_member(value: &str, field: &str) -> Result<(String, ListEntry), Rejection> {
    let dict: Dictionary = Parser::new(value)
        .with_version(Version::Rfc8941)
        .parse()
        .map_err(|e| Rejection::malformed(format!("{field} does not parse: {e}")))?;
    if dict.serialize().as_deref() != Some(value) {
        return Err(Rejection::malformed(format!(
            "{field} is not in canonical form (duplicate keys or parameters, or extra whitespace)"
        )));
    }
    if dict.len() != 1 {
        return Err(Rejection::malformed(format!(
            "{field} holds {} signatures; exactly one is accepted",
            dict.len()
        )));
    }
    let (label, entry) = dict.into_iter().next().expect("one member");
    Ok((label.as_str().to_owned(), entry))
}

fn components(list: &InnerList) -> Result<Vec<String>, Rejection> {
    let mut seen = Vec::with_capacity(list.items.len());
    for item in &list.items {
        let BareItem::String(name) = &item.bare_item else {
            return Err(Rejection::malformed("a covered component is not a string"));
        };
        if !item.params.is_empty() {
            return Err(Rejection::malformed(
                "component parameters are not accepted by this profile",
            ));
        }
        let name = name.as_str();
        if name.is_empty() || name.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err(Rejection::malformed(format!(
                "component name {name:?} is not lower-case"
            )));
        }
        if seen.iter().any(|s: &String| s == name) {
            return Err(Rejection::malformed(format!(
                "component {name:?} is covered twice"
            )));
        }
        seen.push(name.to_owned());
    }
    if seen.is_empty() {
        return Err(Rejection::malformed("the signature covers no components"));
    }
    Ok(seen)
}

fn params_of(params: &Parameters) -> Result<SignatureParams, Rejection> {
    if let Some((unknown, _)) = params
        .iter()
        .find(|(k, _)| !PARAM_NAMES.contains(&k.as_str()))
    {
        return Err(Rejection::malformed(format!(
            "signature parameter {:?} is not accepted",
            unknown.as_str()
        )));
    }
    let get = |name: &str| {
        params
            .iter()
            .find(|(k, _)| k.as_str() == name)
            .map(|(_, v)| v)
            .ok_or_else(|| Rejection::malformed(format!("signature parameter {name} is missing")))
    };
    let integer = |name: &str| -> Result<i64, Rejection> {
        get(name)?
            .as_integer()
            .map(i64::from)
            .ok_or_else(|| Rejection::malformed(format!("{name} is not an integer")))
    };
    let string = |name: &str| -> Result<String, Rejection> {
        get(name)?
            .as_string()
            .map(|s| s.as_str().to_owned())
            .ok_or_else(|| Rejection::malformed(format!("{name} is not a string")))
    };
    Ok(SignatureParams {
        created: integer("created")?,
        expires: integer("expires")?,
        keyid: string("keyid")?,
        alg: string("alg")?,
        nonce: string("nonce")?,
        tag: string("tag")?,
    })
}

fn key(name: &str) -> Result<Key, Rejection> {
    Key::from_string(name.to_owned()).map_err(|(e, _)| {
        Rejection::malformed(format!("{name:?} is not a structured-field key: {e}"))
    })
}

fn sf_string(value: &str) -> Result<BareItem, Rejection> {
    sfv::String::from_string(value.to_owned())
        .map(BareItem::String)
        .map_err(|(e, _)| {
            Rejection::malformed(format!("{value:?} is not a structured-field string: {e}"))
        })
}

fn int(value: i64) -> Result<sfv::Integer, Rejection> {
    sfv::Integer::try_from(value)
        .map_err(|e| Rejection::malformed(format!("{value} is out of range: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> SignatureParams {
        SignatureParams {
            created: 1_893_456_000,
            expires: 1_893_456_120,
            keyid: "arkavo-demo-01".into(),
            alg: "ed25519".into(),
            nonce: "unique-request-nonce".into(),
            tag: "agent-browser-auth".into(),
        }
    }

    #[test]
    fn serializes_the_spec_example_and_parses_it_back() {
        let comps = ["@authority", "@path", "@method", "@target-uri"];
        let (input, sp) = signature_input("tap", &comps, &params()).unwrap();
        assert_eq!(
            input,
            r#"tap=("@authority" "@path" "@method" "@target-uri");created=1893456000;expires=1893456120;nonce="unique-request-nonce";alg="ed25519";keyid="arkavo-demo-01";tag="agent-browser-auth""#
        );
        assert_eq!(sp, &input[4..]);
        let sig = signature("tap", &[1, 2, 3]).unwrap();
        assert_eq!(sig, "tap=:AQID:");
        let parsed = parse(&input, &sig).unwrap();
        assert_eq!(parsed.label, "tap");
        assert_eq!(parsed.components, comps);
        assert_eq!(parsed.params, params());
        assert_eq!(parsed.signature_params, sp);
        assert_eq!(parsed.signature, vec![1, 2, 3]);
    }

    fn input_with(params: &str) -> String {
        format!(r#"tap=("@path");{params}"#)
    }

    const ALL: &str = r#"created=1;expires=2;keyid="k";alg="ed25519";nonce="n";tag="t""#;

    #[test]
    fn rejects_ambiguous_or_non_canonical_fields() {
        let sig = "tap=:AQID:";
        let cases = [
            // a duplicate parameter: the later one would silently win
            input_with(&format!("{ALL};nonce=\"m\"")),
            // two signatures
            format!(r#"tap=("@path");{ALL}, other=("@path");{ALL}"#),
            // extra whitespace
            format!(r#"tap=( "@path");{ALL}"#),
            // label mismatch
            format!(r#"other=("@path");{ALL}"#),
            // camel-case keyId is not a structured-field key at all
            input_with(r#"created=1;expires=2;keyId="k";alg="ed25519";nonce="n";tag="t""#),
            // an unknown parameter
            input_with(&format!("{ALL};extra=1")),
            // a missing parameter
            input_with(r#"created=1;expires=2;keyid="k";alg="ed25519";nonce="n""#),
            // wrong types
            input_with(r#"created="1";expires=2;keyid="k";alg="ed25519";nonce="n";tag="t""#),
            input_with(r#"created=1;expires=2;keyid=k;alg="ed25519";nonce="n";tag="t""#),
            // components: duplicate, upper-case, parameterized, not a string, none
            format!(r#"tap=("@path" "@path");{ALL}"#),
            format!(r#"tap=("Content-Type");{ALL}"#),
            format!(r#"tap=("content-digest";sf);{ALL}"#),
            format!(r#"tap=(path);{ALL}"#),
            format!(r#"tap=();{ALL}"#),
            // not an inner list
            format!(r#"tap="@path";{ALL}"#),
            "tap=(".to_owned(),
        ];
        for input in cases {
            let err = parse(&input, sig).unwrap_err();
            assert_eq!(err.kind, crate::RejectKind::Malformed, "{input}: {err}");
        }
    }

    #[test]
    fn rejects_bad_signature_members() {
        let input = input_with(ALL);
        for sig in [
            "tap=AQID",
            "tap=:AQID:;x=1",
            "tap=:AQID:, b=:AQID:",
            "tap=:AQ ID:",
        ] {
            assert!(parse(&input, sig).is_err(), "{sig}");
        }
        assert!(parse(&input, "tap=:AQID:").is_ok());
    }

    #[test]
    fn refuses_values_that_cannot_be_serialized() {
        let mut p = params();
        p.keyid = "caf\u{e9}".into();
        assert!(signature_input("tap", &["@path"], &p).is_err());
        assert!(signature_input("Tap", &["@path"], &params()).is_err());
        assert!(signature("Tap", &[1]).is_err());
    }
}
