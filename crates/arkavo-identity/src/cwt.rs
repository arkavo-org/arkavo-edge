//! Extract `sub` from an access CWT. Never log the token.

use crate::error::IdentityError;
use base64::Engine;
use ciborium::value::Value;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;
const NO_SUB: &str = "access token has no sub";

pub fn sub(access_token: &str) -> Result<String, IdentityError> {
    let bytes = B64.decode(access_token.trim()).map_err(|_| no_sub())?;
    let value: Value = ciborium::from_reader(bytes.as_slice()).map_err(|_| no_sub())?;
    let claims = claims_map(value).ok_or_else(no_sub)?;
    claim_sub(&claims).ok_or_else(no_sub)
}

fn no_sub() -> IdentityError {
    IdentityError::Token(NO_SUB.into())
}

fn unwrap_tags(mut value: Value) -> Value {
    while let Value::Tag(_, inner) = value {
        value = *inner;
    }
    value
}

fn claims_map(value: Value) -> Option<Vec<(Value, Value)>> {
    match unwrap_tags(value) {
        Value::Map(map) => Some(map),
        Value::Array(items) if items.len() == 4 => payload_map(items.into_iter().nth(2)?),
        Value::Bytes(bytes) => {
            let inner: Value = ciborium::from_reader(bytes.as_slice()).ok()?;
            claims_map(inner)
        }
        _ => None,
    }
}

fn payload_map(payload: Value) -> Option<Vec<(Value, Value)>> {
    match unwrap_tags(payload) {
        Value::Map(map) => Some(map),
        Value::Bytes(bytes) => {
            let inner: Value = ciborium::from_reader(bytes.as_slice()).ok()?;
            match unwrap_tags(inner) {
                Value::Map(map) => Some(map),
                _ => None,
            }
        }
        _ => None,
    }
}

fn claim_sub(map: &[(Value, Value)]) -> Option<String> {
    for (key, value) in map {
        if matches!(key, Value::Integer(i) if i128::from(*i) == 2)
            && let Value::Text(text) = value
            && !text.is_empty()
        {
            return Some(text.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::sub;
    use crate::error::IdentityError;
    use base64::Engine;
    use ciborium::value::Value;

    const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

    fn tagged_cose_sign1_with_sub(sub: &str) -> String {
        let claims = Value::Map(vec![(
            Value::Integer(2.into()),
            Value::Text(sub.to_string()),
        )]);
        let mut payload = Vec::new();
        ciborium::into_writer(&claims, &mut payload).unwrap();
        let cose = Value::Array(vec![
            Value::Bytes(vec![]),
            Value::Map(vec![]),
            Value::Bytes(payload),
            Value::Bytes(vec![]),
        ]);
        let tagged = Value::Tag(61, Box::new(cose));
        let mut bytes = Vec::new();
        ciborium::into_writer(&tagged, &mut bytes).unwrap();
        assert_eq!(&bytes[..2], &[0xD8, 0x3D], "CWT tag 61 is 0xD8 0x3D");
        B64.encode(bytes)
    }

    #[test]
    fn sub_reads_rfc8392_claim_from_tagged_cose_sign1() {
        let token = tagged_cose_sign1_with_sub("arkavo:test-sub");
        assert_eq!(sub(&token).unwrap(), "arkavo:test-sub");
    }

    #[test]
    fn garbage_access_token_has_no_sub() {
        match sub("not-a-cwt") {
            Err(IdentityError::Token(msg)) => assert_eq!(msg, "access token has no sub"),
            other => panic!("expected Token(\"access token has no sub\"), got {other:?}"),
        }
    }
}
