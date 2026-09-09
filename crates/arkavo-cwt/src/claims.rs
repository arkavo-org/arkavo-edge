//! CWT claim decoding.
//!
//! authnz-rs emits the RFC 8392 integer claim keys (`1` iss, `2` sub, `3` aud,
//! `4` exp, `6` iat) alongside Arkavo's own text-keyed claims. `nbf` (`5`) is
//! never emitted, so it is not required here.

use crate::CwtError;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ciborium::Value;

/// The subset of the agent CWT's claims the edge acts on.
#[derive(Debug, Clone, PartialEq)]
pub struct Claims {
    pub iss: String,
    pub sub: String,
    pub aud: Vec<String>,
    pub exp: i64,
    pub iat: i64,
    pub account_id: Option<String>,
    pub roles: Vec<String>,
    pub entitlements: Vec<String>,
    /// Subjects named by the `act` claim — the human principals the agent acts
    /// for. Absent entirely when no actors are configured.
    pub actors: Vec<String>,
    pub npe: Option<serde_json::Value>,
}

impl Claims {
    pub(crate) fn from_cbor(payload: &[u8]) -> Result<Self, CwtError> {
        let value: Value =
            ciborium::from_reader(payload).map_err(|e| CwtError::Claims(e.to_string()))?;
        let Value::Map(entries) = value else {
            return Err(CwtError::Claims("payload is not a CBOR map".into()));
        };

        let mut iss = None;
        let mut sub = None;
        let mut aud = Vec::new();
        let mut exp = None;
        let mut iat = None;
        let mut account_id = None;
        let mut roles = Vec::new();
        let mut entitlements = Vec::new();
        let mut actors = Vec::new();
        let mut npe = None;

        for (key, value) in entries {
            match &key {
                Value::Integer(i) => match i128::from(*i) {
                    1 => iss = Some(text(&value, "iss")?),
                    2 => sub = Some(text(&value, "sub")?),
                    3 => aud = audience(&value)?,
                    4 => exp = Some(integer(&value, "exp")?),
                    6 => iat = Some(integer(&value, "iat")?),
                    _ => {}
                },
                Value::Text(label) => match label.as_str() {
                    "arkavo_account_id" => account_id = Some(text(&value, "arkavo_account_id")?),
                    "arkavo_roles" => roles = text_array(&value, "arkavo_roles")?,
                    "arkavo_entitlements" => {
                        entitlements = text_array(&value, "arkavo_entitlements")?;
                    }
                    "arkavo_npe" => npe = Some(to_json(&value)?),
                    "act" => actors = actor_subjects(&value)?,
                    _ => {}
                },
                _ => {}
            }
        }

        Ok(Self {
            iss: iss.ok_or_else(|| CwtError::Claims("missing iss".into()))?,
            sub: sub.ok_or_else(|| CwtError::Claims("missing sub".into()))?,
            aud,
            exp: exp.ok_or_else(|| CwtError::Claims("missing exp".into()))?,
            iat: iat.ok_or_else(|| CwtError::Claims("missing iat".into()))?,
            account_id,
            roles,
            entitlements,
            actors,
            npe,
        })
    }
}

fn text(value: &Value, claim: &str) -> Result<String, CwtError> {
    value
        .as_text()
        .map(str::to_owned)
        .ok_or_else(|| CwtError::Claims(format!("{claim} is not a text string")))
}

fn integer(value: &Value, claim: &str) -> Result<i64, CwtError> {
    let Value::Integer(i) = value else {
        return Err(CwtError::Claims(format!("{claim} is not an integer")));
    };
    i64::try_from(*i).map_err(|_| CwtError::Claims(format!("{claim} does not fit in i64")))
}

/// `aud` is a CBOR array for agent tokens and a bare text string for other
/// token types; both are legal per RFC 8392.
fn audience(value: &Value) -> Result<Vec<String>, CwtError> {
    match value {
        Value::Text(t) => Ok(vec![t.clone()]),
        Value::Array(_) => text_array(value, "aud"),
        _ => Err(CwtError::Claims("aud is not a text string or array".into())),
    }
}

fn text_array(value: &Value, claim: &str) -> Result<Vec<String>, CwtError> {
    let Value::Array(items) = value else {
        return Err(CwtError::Claims(format!("{claim} is not an array")));
    };
    items.iter().map(|item| text(item, claim)).collect()
}

/// `act` is an array of `{"sub": "<actor>"}` maps.
fn actor_subjects(value: &Value) -> Result<Vec<String>, CwtError> {
    let Value::Array(items) = value else {
        return Err(CwtError::Claims("act is not an array".into()));
    };
    items
        .iter()
        .map(|item| {
            let Value::Map(fields) = item else {
                return Err(CwtError::Claims("act entry is not a map".into()));
            };
            fields
                .iter()
                .find(|(k, _)| k.as_text() == Some("sub"))
                .ok_or_else(|| CwtError::Claims("act entry has no sub".into()))
                .and_then(|(_, v)| text(v, "act.sub"))
        })
        .collect()
}

/// Render `arkavo_npe` as JSON so callers can inspect the delegation chain with
/// the same tooling they use for the rest of the identity plane. Byte strings
/// become base64url text, the only lossless rendering JSON allows.
fn to_json(value: &Value) -> Result<serde_json::Value, CwtError> {
    Ok(match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Integer(i) => serde_json::Value::Number(
            i64::try_from(*i)
                .map_err(|_| CwtError::Claims("arkavo_npe integer does not fit in i64".into()))?
                .into(),
        ),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .ok_or_else(|| CwtError::Claims("arkavo_npe float is not finite".into()))?,
        Value::Text(t) => serde_json::Value::String(t.clone()),
        Value::Bytes(b) => serde_json::Value::String(URL_SAFE_NO_PAD.encode(b)),
        Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(to_json).collect::<Result<_, _>>()?)
        }
        Value::Map(fields) => {
            let mut object = serde_json::Map::with_capacity(fields.len());
            for (key, value) in fields {
                let key = match key {
                    Value::Text(t) => t.clone(),
                    Value::Integer(i) => i128::from(*i).to_string(),
                    _ => {
                        return Err(CwtError::Claims(
                            "arkavo_npe map key is not text or integer".into(),
                        ));
                    }
                };
                object.insert(key, to_json(value)?);
            }
            serde_json::Value::Object(object)
        }
        Value::Tag(_, inner) => to_json(inner)?,
        _ => {
            return Err(CwtError::Claims(
                "unsupported CBOR value in arkavo_npe".into(),
            ));
        }
    })
}
