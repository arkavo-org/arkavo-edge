//! RFC 8785 JSON Canonicalization Scheme (JCS)
//!
//! Produces deterministic JSON serialization for signing:
//! - Object keys sorted lexicographically by UTF-16 code units
//! - No whitespace between tokens
//! - Numbers serialized per ES2015/IEEE 754 rules
//! - Strings with minimal escape sequences

use serde_json::Value;

/// Canonicalize a JSON value per RFC 8785 (JCS).
pub fn canonicalize(value: &Value) -> String {
    let mut output = String::new();
    write_canonical(value, &mut output);
    output
}

/// Canonicalize a serializable value by first converting to serde_json::Value.
pub fn canonicalize_serializable<T: serde::Serialize>(
    value: &T,
) -> Result<String, serde_json::Error> {
    let json_value = serde_json::to_value(value)?;
    Ok(canonicalize(&json_value))
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => write_number(n, out),
        Value::String(s) => write_string(s, out),
        Value::Array(arr) => write_array(arr, out),
        Value::Object(obj) => write_object(obj, out),
    }
}

fn write_number(n: &serde_json::Number, out: &mut String) {
    // RFC 8785: integers render without decimal point;
    // floats use ES2015 Number.toString() rules.
    // serde_json::Number already handles this correctly for integers.
    if let Some(i) = n.as_i64() {
        out.push_str(&i.to_string());
    } else if let Some(u) = n.as_u64() {
        out.push_str(&u.to_string());
    } else if let Some(f) = n.as_f64() {
        // ES2015 rules: shortest representation that round-trips
        out.push_str(&format_float(f));
    }
}

fn format_float(f: f64) -> String {
    if f == 0.0 {
        return "0".to_string();
    }
    // Use Rust's default float formatting which matches ES2015 in most cases
    let s = format!("{f}");
    // Strip trailing zeros after decimal point, then trailing decimal point
    if s.contains('.') {
        let trimmed = s.trim_end_matches('0');
        let trimmed = trimmed.trim_end_matches('.');
        trimmed.to_string()
    } else {
        s
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < '\u{0020}' => {
                // Control characters: \u00XX
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_array(arr: &[Value], out: &mut String) {
    out.push('[');
    for (i, v) in arr.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_canonical(v, out);
    }
    out.push(']');
}

fn write_object(obj: &serde_json::Map<String, Value>, out: &mut String) {
    // RFC 8785: sort keys by UTF-16 code units
    let mut keys: Vec<&String> = obj.keys().collect();
    keys.sort_by(|a, b| cmp_utf16(a, b));

    out.push('{');
    for (i, key) in keys.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_string(key, out);
        out.push(':');
        write_canonical(&obj[*key], out);
    }
    out.push('}');
}

/// Compare strings by UTF-16 code unit ordering (RFC 8785 requirement).
fn cmp_utf16(a: &str, b: &str) -> std::cmp::Ordering {
    let a_units: Vec<u16> = a.encode_utf16().collect();
    let b_units: Vec<u16> = b.encode_utf16().collect();
    a_units.cmp(&b_units)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn null_bool_values() {
        assert_eq!(canonicalize(&json!(null)), "null");
        assert_eq!(canonicalize(&json!(true)), "true");
        assert_eq!(canonicalize(&json!(false)), "false");
    }

    #[test]
    fn integer_values() {
        assert_eq!(canonicalize(&json!(0)), "0");
        assert_eq!(canonicalize(&json!(42)), "42");
        assert_eq!(canonicalize(&json!(-1)), "-1");
        assert_eq!(canonicalize(&json!(1000)), "1000");
    }

    #[test]
    fn string_values() {
        assert_eq!(canonicalize(&json!("hello")), "\"hello\"");
        assert_eq!(canonicalize(&json!("")), "\"\"");
    }

    #[test]
    fn string_escaping() {
        assert_eq!(canonicalize(&json!("a\"b")), "\"a\\\"b\"");
        assert_eq!(canonicalize(&json!("a\\b")), "\"a\\\\b\"");
        assert_eq!(canonicalize(&json!("a\nb")), "\"a\\nb\"");
        assert_eq!(canonicalize(&json!("a\tb")), "\"a\\tb\"");
    }

    #[test]
    fn array_values() {
        assert_eq!(canonicalize(&json!([])), "[]");
        assert_eq!(canonicalize(&json!([1, 2, 3])), "[1,2,3]");
        assert_eq!(canonicalize(&json!(["a", "b"])), "[\"a\",\"b\"]");
    }

    #[test]
    fn object_key_sorting() {
        let val = json!({"b": 2, "a": 1, "c": 3});
        assert_eq!(canonicalize(&val), "{\"a\":1,\"b\":2,\"c\":3}");
    }

    #[test]
    fn nested_objects_sorted() {
        let val = json!({"z": {"b": 2, "a": 1}, "a": 0});
        assert_eq!(canonicalize(&val), "{\"a\":0,\"z\":{\"a\":1,\"b\":2}}");
    }

    #[test]
    fn empty_object() {
        assert_eq!(canonicalize(&json!({})), "{}");
    }

    #[test]
    fn no_whitespace() {
        let val = json!({"key": [1, 2, {"nested": true}]});
        let result = canonicalize(&val);
        assert!(!result.contains(' '));
        assert!(!result.contains('\n'));
    }

    #[test]
    fn trust_score_canonical() {
        let val = json!({
            "schema_version": "0.1.0",
            "subject_id": "did:key:z6MkTest",
            "provider_id": "did:key:z6MkProvider",
            "score": {
                "composite": 742,
                "dimensions": {
                    "performance": {"value": 780, "confidence": 0.72, "evidence_count": 156}
                }
            }
        });
        let result = canonicalize(&val);
        // Keys must be sorted at every level
        assert!(result.starts_with("{\"provider_id\":"));
        assert!(result.contains("\"schema_version\":\"0.1.0\""));
    }

    #[test]
    fn canonicalize_serializable_works() {
        #[derive(serde::Serialize)]
        struct Test {
            b: i32,
            a: i32,
        }
        let t = Test { b: 2, a: 1 };
        let result = canonicalize_serializable(&t).unwrap();
        assert_eq!(result, "{\"a\":1,\"b\":2}");
    }

    #[test]
    fn utf16_key_ordering() {
        // RFC 8785 requires UTF-16 code unit ordering
        let val = json!({"b": 1, "a": 2, "ab": 3});
        let result = canonicalize(&val);
        assert_eq!(result, "{\"a\":2,\"ab\":3,\"b\":1}");
    }

    #[test]
    fn control_character_escaping() {
        let val = json!({"key": "\u{0001}\u{001f}"});
        let result = canonicalize(&val);
        assert!(result.contains("\\u0001"));
        assert!(result.contains("\\u001f"));
    }
}
