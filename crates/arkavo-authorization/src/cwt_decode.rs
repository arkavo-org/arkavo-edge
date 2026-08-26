//! CWT payload decoder: duplicate-key reject and minimum `$token` claims.
#![allow(clippy::redundant_pub_crate)]

use crate::cwt_subject::{Aud, DecodedClaims};
use crate::cwt_verify::CwtError;
use ciborium::value::Value;
use tracing::warn;

pub(crate) fn parse_claims(payload: &[u8]) -> Result<DecodedClaims, CwtError> {
    let value: Value = ciborium::de::from_reader(payload).map_err(|_| CwtError::Malformed)?;
    let Value::Map(entries) = value else {
        return Err(CwtError::Malformed);
    };
    reject_duplicate_keys(&entries)?;

    let mut iss = None;
    let mut sub = None;
    let mut aud = None;
    let mut exp = None;
    let mut iat = None;
    let mut cti = None;
    let mut email = None;
    let mut email_verified = None;
    let mut idp = None;
    let mut arkavo_account_id = None;
    let mut arkavo_roles = None;
    let mut arkavo_entitlements = None;
    let mut client_id = None;
    let mut arkavo_patreon = None;

    for (k, v) in entries {
        match k {
            Value::Integer(key) => match (i128::from(key), v) {
                (1, Value::Text(s)) => iss = Some(s),
                (2, Value::Text(s)) => sub = Some(s),
                (3, Value::Text(s)) => aud = Some(Aud::One(s)),
                (3, Value::Array(a)) => {
                    let parts: Result<Vec<String>, _> = a
                        .into_iter()
                        .map(|x| match x {
                            Value::Text(s) => Ok(s),
                            _ => Err(CwtError::Malformed),
                        })
                        .collect();
                    let parts = parts?;
                    if parts.is_empty() {
                        return Err(CwtError::Malformed);
                    }
                    aud = Some(Aud::Many(parts));
                }
                (4, Value::Integer(n)) => {
                    exp = Some(u64::try_from(i128::from(n)).map_err(|_| CwtError::Malformed)?);
                }
                (6, Value::Integer(n)) => {
                    iat = Some(u64::try_from(i128::from(n)).map_err(|_| CwtError::Malformed)?);
                }
                (7, Value::Bytes(b)) => {
                    if b.len() != 16 {
                        return Err(CwtError::Malformed);
                    }
                    cti = Some(b);
                }
                (8, _) => {}
                _ => {}
            },
            Value::Text(key) => match (key.as_str(), v) {
                ("email", Value::Text(s)) => email = Some(s),
                ("email_verified", Value::Bool(b)) => email_verified = Some(b),
                ("idp", Value::Text(s)) => idp = Some(s),
                ("arkavo_account_id", Value::Text(s)) => arkavo_account_id = Some(s),
                ("client_id", Value::Text(s)) => client_id = Some(s),
                ("arkavo_roles", Value::Array(a)) => {
                    arkavo_roles = Some(text_array(a)?);
                }
                ("arkavo_entitlements", Value::Array(a)) => {
                    arkavo_entitlements = Some(text_array(a)?);
                }
                ("arkavo_patreon", patreon) => arkavo_patreon = Some(cbor_to_json(&patreon)),
                _ => {}
            },
            _ => return Err(CwtError::Malformed),
        }
    }

    Ok(DecodedClaims {
        iss: iss.ok_or(CwtError::MissingClaim("iss"))?,
        sub: sub.ok_or(CwtError::MissingClaim("sub"))?,
        aud: aud.ok_or(CwtError::MissingClaim("aud"))?,
        exp: exp.ok_or(CwtError::MissingClaim("exp"))?,
        iat: iat.ok_or(CwtError::MissingClaim("iat"))?,
        cti: cti.ok_or(CwtError::MissingClaim("cti"))?,
        email,
        email_verified,
        idp,
        arkavo_account_id,
        arkavo_roles,
        arkavo_entitlements,
        client_id,
        arkavo_patreon,
    })
}

fn text_array(a: Vec<Value>) -> Result<Vec<String>, CwtError> {
    a.into_iter()
        .map(|x| match x {
            Value::Text(s) => Ok(s),
            _ => Err(CwtError::Malformed),
        })
        .collect()
}

fn reject_duplicate_keys(entries: &[(Value, Value)]) -> Result<(), CwtError> {
    let mut seen_ints = Vec::new();
    let mut seen_strs = Vec::new();
    for (k, _) in entries {
        match k {
            Value::Integer(i) => {
                let n: i128 = (*i).into();
                if seen_ints.contains(&n) {
                    return Err(CwtError::DuplicateKey);
                }
                seen_ints.push(n);
            }
            Value::Text(s) => {
                if seen_strs.iter().any(|t: &String| t == s) {
                    return Err(CwtError::DuplicateKey);
                }
                seen_strs.push(s.clone());
            }
            _ => return Err(CwtError::Malformed),
        }
    }
    Ok(())
}

fn cbor_to_json(v: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::Text(s) => J::String(s.clone()),
        Value::Integer(n) => {
            let i: i128 = (*n).into();
            i64::try_from(i)
                .map(J::from)
                .or_else(|_| u64::try_from(i).map(J::from))
                .unwrap_or_else(|_| J::String(i.to_string()))
        }
        Value::Bool(b) => J::Bool(*b),
        Value::Null => J::Null,
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(J::Number)
            .unwrap_or(J::Null),
        Value::Bytes(b) => {
            use base64::Engine;
            J::String(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b))
        }
        Value::Array(a) => J::Array(a.iter().map(cbor_to_json).collect()),
        Value::Map(m) => {
            let mut obj = serde_json::Map::new();
            for (k, val) in m {
                if let Value::Text(key) = k {
                    obj.insert(key.clone(), cbor_to_json(val));
                }
            }
            J::Object(obj)
        }
        other => {
            warn!("cbor_to_json: unexpected CBOR type: {other:?}");
            J::String(format!("{other:?}"))
        }
    }
}
