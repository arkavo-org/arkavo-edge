//! BLAKE3 content addressing in the `b3:<64-hex>` form the eval contract uses.
//! (The existing `arkavo_swarmkit::canonical::content_hash` emits a different
//! `blake3:<base64url>` form and is left untouched.)

use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum DigestError {
    #[error("digest must start with 'b3:'")]
    MissingPrefix,
    #[error("digest hex must be 64 chars, got {0}")]
    BadLength(usize),
    #[error("invalid hex in digest")]
    BadHex,
}

/// Hash bytes and return `b3:<64 lowercase hex>`.
pub fn b3_hex(bytes: &[u8]) -> String {
    format!("b3:{}", blake3::hash(bytes).to_hex())
}

/// Parse a `b3:<hex>` string into 32 raw bytes.
pub fn parse_b3(s: &str) -> Result<[u8; 32], DigestError> {
    let hex = s.strip_prefix("b3:").ok_or(DigestError::MissingPrefix)?;
    if hex.len() != 64 {
        return Err(DigestError::BadLength(hex.len()));
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16).ok_or(DigestError::BadHex)?;
        let lo = (chunk[1] as char).to_digit(16).ok_or(DigestError::BadHex)?;
        out[i] = ((hi << 4) | lo) as u8;
    }
    Ok(out)
}

/// True if `bytes` hash to `expected` (a `b3:<hex>` string).
pub fn verify_b3(bytes: &[u8], expected: &str) -> bool {
    b3_hex(bytes) == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_verify() {
        let d = b3_hex(b"hello");
        assert!(d.starts_with("b3:"));
        assert_eq!(d.len(), 3 + 64);
        assert!(verify_b3(b"hello", &d));
        assert!(!verify_b3(b"world", &d));
        let raw = parse_b3(&d).unwrap();
        assert_eq!(raw, *blake3::hash(b"hello").as_bytes());
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert_eq!(parse_b3("xx:abc"), Err(DigestError::MissingPrefix));
        assert_eq!(parse_b3("b3:abc"), Err(DigestError::BadLength(3)));
        let bad = format!("b3:{}", "z".repeat(64));
        assert_eq!(parse_b3(&bad), Err(DigestError::BadHex));
    }
}
