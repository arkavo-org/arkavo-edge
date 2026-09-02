//! Canonical JSON encoding for tool-call argument hashing.
//!
//! Canonicalization rules (see `docs/permit-cwt-schema.md`):
//! - UTF-8 output with no insignificant whitespace.
//! - Object keys sorted by Unicode code point; no duplicate keys (the input
//!   `serde_json::Value` cannot hold duplicates).
//! - Strings escaped per RFC 8259 section 7 (short escapes where defined,
//!   `\u00XX` for other control characters).
//! - Numbers rendered with `serde_json::Number`'s shortest representation.

use crate::error::PermitError;
use crate::hash::HashAlgorithm;
use serde_json::Value;

/// Encode a JSON value in canonical form.
pub fn canonical_json(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write_canonical(value, &mut out);
    out
}

/// Hash canonicalized tool-call arguments with the selected algorithm.
pub fn argument_hash(arguments: &Value, algorithm: HashAlgorithm) -> Vec<u8> {
    algorithm.digest(&canonical_json(arguments))
}

fn write_canonical(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Number(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::String(s) => write_json_string(s, out),
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical(item, out);
            }
            out.push(b']');
        }
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            out.push(b'{');
            for (i, (key, val)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_json_string(key, out);
                out.push(b':');
                write_canonical(val, out);
            }
            out.push(b'}');
        }
    }
}

fn write_json_string(s: &str, out: &mut Vec<u8>) {
    out.push(b'"');
    for ch in s.chars() {
        match ch {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\u{08}' => out.extend_from_slice(b"\\b"),
            '\u{0C}' => out.extend_from_slice(b"\\f"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            c if (c as u32) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes());
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

/// Parse raw JSON text and return its canonical encoding.
///
/// Used by integrators that receive arguments as a JSON string rather than a
/// parsed value; fails closed on invalid JSON.
pub fn canonicalize_json_text(text: &str) -> Result<Vec<u8>, PermitError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|_| PermitError::MalformedClaim("arguments are not valid JSON"))?;
    Ok(canonical_json(&value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn object_keys_are_sorted() {
        let value = json!({"b": 2, "a": 1, "c": {"z": 1, "y": 2}});
        assert_eq!(
            canonical_json(&value),
            br#"{"a":1,"b":2,"c":{"y":2,"z":1}}"#
        );
    }

    #[test]
    fn no_whitespace_anywhere() {
        let value = json!({"a": [1, 2, {"b": "c"}], "d": true, "e": null});
        let canonical = canonical_json(&value);
        assert_eq!(canonical, br#"{"a":[1,2,{"b":"c"}],"d":true,"e":null}"#);
    }

    #[test]
    fn strings_escaped_per_rfc8259() {
        let value = json!("quote\" backslash\\ newline\n tab\t nul\u{0}");
        assert_eq!(
            canonical_json(&value),
            br#""quote\" backslash\\ newline\n tab\t nul\u0000""#
        );
    }

    #[test]
    fn unicode_strings_stay_utf8() {
        let value = json!({"k": "héllo wörld ✓"});
        let canonical = canonical_json(&value);
        assert_eq!(canonical, "{\"k\":\"héllo wörld ✓\"}".as_bytes());
    }

    #[test]
    fn argument_hash_is_stable_across_key_order() {
        let a = json!({"x": 1, "y": [true, null]});
        let b = json!({"y": [true, null], "x": 1});
        assert_eq!(
            argument_hash(&a, HashAlgorithm::Sha256),
            argument_hash(&b, HashAlgorithm::Sha256)
        );
        assert_eq!(
            argument_hash(&a, HashAlgorithm::Blake3),
            argument_hash(&b, HashAlgorithm::Blake3)
        );
    }

    #[test]
    fn different_arguments_hash_differently() {
        let a = json!({"path": "/tmp/a"});
        let b = json!({"path": "/tmp/b"});
        assert_ne!(
            argument_hash(&a, HashAlgorithm::Sha256),
            argument_hash(&b, HashAlgorithm::Sha256)
        );
    }

    #[test]
    fn canonicalize_json_text_roundtrip_and_rejects_garbage() {
        assert_eq!(
            canonicalize_json_text("{ \"b\": 1, \"a\": 2 }").unwrap(),
            br#"{"a":2,"b":1}"#
        );
        assert!(canonicalize_json_text("{not json").is_err());
    }
}
