use thiserror::Error;

#[derive(Error, Debug)]
pub enum PermitError {
    #[error("CBOR serialization failed: {0}")]
    CborSerialize(String),
    #[error("CBOR deserialization failed: {0}")]
    CborDeserialize(String),
    #[error("COSE processing failed: {0}")]
    Cose(String),
    #[error("missing required claim: {0}")]
    MissingClaim(&'static str),
    #[error("malformed claim: {0}")]
    MalformedClaim(&'static str),
    #[error("invalid confirmation key: {0}")]
    InvalidConfirmationKey(String),
    #[error("unsupported signing algorithm: {0}")]
    UnsupportedAlgorithm(String),
    #[error("confirmation key does not match the COSE header algorithm")]
    KeyAlgorithmMismatch,
    #[error("signature verification failed")]
    InvalidSignature,
    #[error("permit is not yet valid (nbf {nbf} > now {now})")]
    NotYetValid { nbf: i64, now: i64 },
    #[error("permit has expired (exp {exp} <= now {now})")]
    Expired { exp: i64, now: i64 },
    #[error("permit was issued in the future (iat {iat} > now {now})")]
    IssuedInFuture { iat: i64, now: i64 },
    #[error("invocation does not match the permit binding: {0}")]
    BindingMismatch(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_is_descriptive() {
        let e = PermitError::Expired { exp: 100, now: 200 };
        assert!(e.to_string().contains("100"));
        let e = PermitError::MissingClaim("iss");
        assert!(e.to_string().contains("iss"));
    }
}
