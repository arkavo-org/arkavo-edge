use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;
const B32: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

impl Pkce {
    pub fn generate() -> Self {
        let verifier = random_b64url_32();
        let state = random_b64url_32();
        let challenge = Self::challenge_s256(&verifier);
        Self {
            verifier,
            challenge,
            state,
        }
    }

    pub fn challenge_s256(verifier: &str) -> String {
        let digest = Sha256::digest(verifier.as_bytes());
        B64.encode(digest)
    }

    pub fn pairing_code(state: &str) -> String {
        let digest = Sha256::digest(state.as_bytes());
        let encoded = encode_base32_5(&digest[..5]);
        format!("{}-{}", &encoded[..4], &encoded[4..])
    }
}

fn random_b64url_32() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    B64.encode(bytes)
}

fn encode_base32_5(bytes: &[u8]) -> String {
    // 5 bytes = 40 bits = 8 base32 chars. No padding.
    debug_assert_eq!(bytes.len(), 5);
    let n = u64::from(bytes[0]) << 32
        | u64::from(bytes[1]) << 24
        | u64::from(bytes[2]) << 16
        | u64::from(bytes[3]) << 8
        | u64::from(bytes[4]);
    let mut out = [0u8; 8];
    for (i, slot) in out.iter_mut().enumerate() {
        let shift = 35 - i * 5;
        *slot = B32[((n >> shift) & 0x1f) as usize];
    }
    String::from_utf8(out.to_vec()).expect("base32 alphabet is ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc7636_appendix_b_s256_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            Pkce::challenge_s256(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn pairing_code_is_base32_of_first_five_sha256_bytes() {
        assert_eq!(Pkce::pairing_code("test-state"), "DFRA-BZGI");
    }

    #[test]
    fn generate_uses_unpadded_base64url_of_32_bytes() {
        let p = Pkce::generate();
        assert_eq!(p.verifier.len(), 43);
        assert_eq!(p.state.len(), 43);
        assert!(
            !p.verifier.contains('=') && !p.verifier.contains('+') && !p.verifier.contains('/')
        );
        assert_eq!(p.challenge, Pkce::challenge_s256(&p.verifier));
        let code = Pkce::pairing_code(&p.state);
        assert_eq!(code.len(), 9);
        assert_eq!(code.as_bytes()[4], b'-');
        assert!(
            code.chars()
                .all(|c| c == '-' || matches!(c, 'A'..='Z' | '2'..='7'))
        );
    }
}
