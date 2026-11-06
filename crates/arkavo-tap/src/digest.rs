//! `Content-Digest` (RFC 9530).
//!
//! Computed over the exact content bytes, and verified before anything deserializes them.
//! Only SHA-256 and SHA-512 are accepted; a field naming any other algorithm is refused
//! rather than partly checked.

use crate::error::{RejectKind, Rejection};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sfv::{BareItem, Dictionary, FieldType, Item, ListEntry, Parser, Version};
use sha2::{Digest, Sha256, Sha512};

/// The `Content-Digest` value for `content`, using SHA-256.
pub fn content_digest(content: &[u8]) -> String {
    format!("sha-256=:{}:", STANDARD.encode(Sha256::digest(content)))
}

/// Checks a received `Content-Digest` against the content: every listed digest must be a
/// SHA-256 or SHA-512 digest of exactly these bytes.
pub fn verify(field: &str, content: &[u8]) -> Result<(), Rejection> {
    let dict: Dictionary = Parser::new(field)
        .with_version(Version::Rfc8941)
        .parse()
        .map_err(|e| Rejection::malformed(format!("Content-Digest does not parse: {e}")))?;
    if dict.serialize().as_deref() != Some(field) {
        return Err(Rejection::malformed(
            "Content-Digest is not in canonical form",
        ));
    }
    for (alg, entry) in &dict {
        let ListEntry::Item(Item {
            bare_item: BareItem::ByteSequence(claimed),
            params,
        }) = entry
        else {
            return Err(Rejection::malformed(
                "a Content-Digest member is not a byte sequence",
            ));
        };
        if !params.is_empty() {
            return Err(Rejection::malformed(
                "a Content-Digest member carries parameters",
            ));
        }
        let actual = match alg.as_str() {
            "sha-256" => Sha256::digest(content).to_vec(),
            "sha-512" => Sha512::digest(content).to_vec(),
            other => {
                return Err(Rejection::malformed(format!(
                    "digest algorithm {other} is not accepted"
                )));
            }
        };
        if *claimed != actual {
            return Err(Rejection::new(
                RejectKind::InvalidSignature,
                format!("{} digest does not match the content", alg.as_str()),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &[u8] = br#"{"hello": "world"}"#;

    #[test]
    fn matches_rfc_9530_examples() {
        assert_eq!(
            content_digest(BODY),
            "sha-256=:X48E9qOokqqrvdts8nOJRJN3OWDUoyWxBf7kbu9DBPE=:"
        );
        verify(&content_digest(BODY), BODY).unwrap();
        let sha512 = "sha-512=:WZDPaVn/7XgHaAy8pmojAkGWoRx2UFChF41A2svX+TaPm+AbwAgBWnrIiYllu7BNNyealdVLvRwEmTHWXvJwew==:";
        verify(sha512, BODY).unwrap();
        verify(&format!("{}, {sha512}", content_digest(BODY)), BODY).unwrap();
    }

    #[test]
    fn modified_content_fails() {
        let err = verify(&content_digest(BODY), br#"{"hello": "world!"}"#).unwrap_err();
        assert_eq!(err.kind, RejectKind::InvalidSignature);
    }

    #[test]
    fn refuses_other_algorithms_and_non_canonical_fields() {
        for field in [
            "md5=:AQID:",
            "sha-256=AQID",
            "sha-256=:X48E9qOokqqrvdts8nOJRJN3OWDUoyWxBf7kbu9DBPE=:;x=1",
            "sha-256=:AAAA:, sha-256=:X48E9qOokqqrvdts8nOJRJN3OWDUoyWxBf7kbu9DBPE=:",
            "sha-256=:X48E9qOokqqrvdts8nOJRJN3OWDUoyWxBf7kbu9DBPE=:,sha-512=:AQID:",
            "(",
        ] {
            let err = verify(field, BODY).unwrap_err();
            assert_eq!(err.kind, RejectKind::Malformed, "{field}");
        }
    }
}
