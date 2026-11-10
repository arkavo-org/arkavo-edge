//! Proof of possession for a market request.
//!
//! Agent tokens are bearer tokens every audience accepts, so the market also
//! wants each request signed by the key the token's `cnf` names. This module
//! is the only place the agent key signs for the market, and it signs only
//! text built here: the first line fixes what the signature can mean, so it
//! can never be taken for a signature over anything else. Nothing outside the
//! client can hand it text to sign.

use arkavo_crypto::AgentKeypair;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

const CONTEXT: &str = "arkavo-market-request/1";

/// The header a signed request carries.
pub(super) const HEADER: &str = "Arkavo-Signature";

fn sha256(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(bytes))
}

/// The text the signature covers: seven lines joined with `\n`, with no
/// trailing newline. `path_and_query` and `token` are the bytes sent; the
/// market checks them as received.
fn signing_input(
    method: &str,
    path_and_query: &str,
    t: u64,
    n: &str,
    body: &[u8],
    token: &str,
) -> String {
    format!(
        "{CONTEXT}\n{}\n{path_and_query}\n{t}\n{n}\n{}\n{}",
        method.to_ascii_uppercase(),
        sha256(body),
        sha256(token.as_bytes()),
    )
}

/// The `Arkavo-Signature` value for one request: `t=…, n=…, s=…`.
///
/// `nonce` must be fresh for every request sent, a retry included: the market
/// refuses one it has seen.
pub(super) fn header(
    keypair: &AgentKeypair,
    method: &str,
    path_and_query: &str,
    body: &[u8],
    token: &str,
    t: u64,
    nonce: [u8; 16],
) -> String {
    let n = URL_SAFE_NO_PAD.encode(nonce);
    let signed = signing_input(method, path_and_query, t, &n, body, token);
    let s = URL_SAFE_NO_PAD.encode(keypair.sign(signed.as_bytes()));
    format!("t={t}, n={n}, s={s}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_crypto::AgentKeypair;

    /// The market's own vector (`the_signed_text_is_the_contract` in its
    /// `market-commerce/src/auth.rs`). If this changes, the two sides no
    /// longer sign and verify the same text.
    #[test]
    fn the_signed_text_matches_the_markets_vector() {
        assert_eq!(
            signing_input(
                "post",
                "/api/requests?x=1",
                1_800_000_000,
                "AAAAAAAAAAAAAAAAAAAAAA",
                b"",
                "tok"
            ),
            "arkavo-market-request/1\nPOST\n/api/requests?x=1\n1800000000\nAAAAAAAAAAAAAAAAAAAAAA\n\
             47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU\nGnZ0607njffhrEOak8P6jjyUV4TU3sn9jjARc4svHWI"
        );
    }

    #[test]
    fn the_header_carries_time_nonce_and_a_signature_over_the_signed_text() {
        let keypair = AgentKeypair::generate();
        let body = br#"{"offer":"o","term":3}"#;

        let header = header(
            &keypair,
            "POST",
            "/api/requests",
            body,
            "tok",
            1_800_000_000,
            [0; 16],
        );

        let parts: Vec<&str> = header.split(", ").collect();
        assert_eq!(parts.len(), 3, "{header}");
        assert_eq!(parts[0], "t=1800000000");
        assert_eq!(parts[1], "n=AAAAAAAAAAAAAAAAAAAAAA");
        let signature = URL_SAFE_NO_PAD
            .decode(parts[2].strip_prefix("s=").unwrap())
            .unwrap();
        assert_eq!(signature.len(), 64);
        let signed = signing_input(
            "POST",
            "/api/requests",
            1_800_000_000,
            "AAAAAAAAAAAAAAAAAAAAAA",
            body,
            "tok",
        );
        keypair
            .public_key()
            .verify(signed.as_bytes(), &signature)
            .unwrap();
        assert!(header.len() <= 512);
    }
}
