//! Canonical-form JSON serialization (JCS, RFC 8785) and BLAKE3 content addressing.
//!
//! Per spec §9.1, `kit.id` is `BLAKE3(canonical_manifest)` where the canonical form
//! excludes the `kit.id` and `provenance.signatures` fields themselves.
//!
//! JCS canonicalization rules applied here:
//! - Object keys sorted lexicographically by Unicode codepoint.
//! - No insignificant whitespace.
//! - UTF-8 output.
//! - Numbers serialized via the JSON-canonicalization rules (RFC 8785 §3.2.2).
//!   We leverage `serde_json::Number`'s default representation, which already
//!   produces the shortest unique representation for f64 and exact representation
//!   for integers — sufficient for SwarmKit canonicalization where numeric fields
//!   are bounded (token counts, weights, thresholds).

use base64::Engine;
use serde_json::Value;

use crate::manifest::Manifest;

/// Produce the JCS-canonical JSON encoding of a `serde_json::Value`.
pub fn canonical_json(value: &Value) -> String {
    let mut buf = String::new();
    write_canonical(value, &mut buf);
    buf
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => write_json_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(k, out);
                out.push(':');
                write_canonical(&map[*k], out);
            }
            out.push('}');
        }
    }
}

/// Write a JSON string with RFC 8785-compliant escaping.
fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Compute the BLAKE3 content hash of an arbitrary canonicalized value.
/// Returns `blake3:<base64url-no-padding-of-32-byte-digest>`.
pub fn content_hash(canonical: &str) -> String {
    let digest = blake3::hash(canonical.as_bytes());
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest.as_bytes());
    format!("blake3:{encoded}")
}

/// Compute the `kit.id` value for a manifest per spec §9.1: BLAKE3 of the canonical
/// form with `kit.id` and `provenance.signatures` excluded.
pub fn kit_id_for(manifest: &Manifest) -> Result<String, serde_json::Error> {
    let mut value = serde_json::to_value(manifest)?;
    strip_id_and_signatures(&mut value);
    let canonical = canonical_json(&value);
    Ok(content_hash(&canonical))
}

fn strip_id_and_signatures(value: &mut Value) {
    if let Value::Object(root) = value {
        if let Some(Value::Object(kit)) = root.get_mut("kit") {
            kit.remove("id");
        }
        if let Some(Value::Object(prov)) = root.get_mut("provenance") {
            prov.remove("signatures");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_sorts_keys() {
        let v = json!({"b": 1, "a": 2});
        assert_eq!(canonical_json(&v), r#"{"a":2,"b":1}"#);
    }

    #[test]
    fn canonical_no_whitespace() {
        let v = json!({"k": [1, 2, 3]});
        assert_eq!(canonical_json(&v), r#"{"k":[1,2,3]}"#);
    }

    #[test]
    fn canonical_escapes_special() {
        let v = json!({"s": "a\nb\tc\"d"});
        assert_eq!(canonical_json(&v), r#"{"s":"a\nb\tc\"d"}"#);
    }

    #[test]
    fn canonical_recurses() {
        let v = json!({"z": {"b": 1, "a": 2}, "a": [{"y": 3, "x": 4}]});
        assert_eq!(
            canonical_json(&v),
            r#"{"a":[{"x":4,"y":3}],"z":{"a":2,"b":1}}"#
        );
    }

    #[test]
    fn content_hash_format() {
        let h = content_hash(r#"{"a":1}"#);
        assert!(h.starts_with("blake3:"));
        // base64url no-pad of 32 bytes = 43 chars
        assert_eq!(h.len(), "blake3:".len() + 43);
    }

    #[test]
    fn content_hash_deterministic() {
        let a = content_hash(r#"{"a":1,"b":2}"#);
        let b = content_hash(r#"{"a":1,"b":2}"#);
        assert_eq!(a, b);
    }
}
