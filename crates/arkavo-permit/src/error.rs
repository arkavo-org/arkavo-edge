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
    #[error("proof-of-possession does not verify under the permit's cnf key")]
    InvalidProof,
}

impl From<arkavo_cwt::CwtError> for PermitError {
    fn from(error: arkavo_cwt::CwtError) -> Self {
        use arkavo_cwt::CwtError as E;
        match error {
            E::BadSignature => Self::InvalidSignature,
            E::KeyAlgorithmMismatch => Self::KeyAlgorithmMismatch,
            E::Key(message) => Self::InvalidConfirmationKey(message),
            E::UnsupportedAlgorithm(message) => Self::UnsupportedAlgorithm(message),
            other => Self::Cose(other.to_string()),
        }
    }
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
