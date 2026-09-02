//! Signing and verification keys for permits, reusing `arkavo-crypto` key
//! types (Ed25519 primary, P-256/ES256 supported) and encoding them as
//! COSE_Key confirmation keys per RFC 8747 via `arkavo-cwt`.

use crate::error::PermitError;
use arkavo_crypto::{AgentKeypair, P256SigningKeypair};
use coset::CoseKey;
use coset::iana::Algorithm;

/// An issuer's permit signing key.
///
/// It fixes the COSE algorithm and the `kid` of the protected header, and
/// nothing else: the `cnf` claim names the presenter, whose
/// [`PermitVerifier`] is a separate argument to [`mint`](crate::mint).
pub enum PermitSigner {
    Ed25519(AgentKeypair),
    P256(P256SigningKeypair),
}

impl PermitSigner {
    pub fn algorithm(&self) -> Algorithm {
        match self {
            Self::Ed25519(_) => Algorithm::EdDSA,
            Self::P256(_) => Algorithm::ES256,
        }
    }

    /// The issuer's public half.
    ///
    /// Verifiers carry these in their `trusted_issuers` list, and
    /// [`issuer_kid`](crate::issuer_kid) digests this key's bytes into the
    /// `kid` that [`mint`](crate::mint) writes into the protected header.
    ///
    /// # Panics
    ///
    /// Panics if `arkavo-crypto` hands out a public key that is not a
    /// canonical 32-byte Ed25519 point or a valid SEC1 P-256 point, which its
    /// key types do not allow.
    pub fn public_key(&self) -> PermitVerifier {
        match self {
            Self::Ed25519(keypair) => {
                let bytes = keypair.public_key().to_bytes();
                // arkavo-crypto only ever hands out well-formed 32-byte Ed25519 keys.
                let raw: [u8; 32] = bytes[..32]
                    .try_into()
                    .expect("Ed25519 public key is 32 bytes");
                let key = ed25519_dalek::VerifyingKey::from_bytes(&raw)
                    .expect("arkavo-crypto Ed25519 keys are canonical");
                PermitVerifier(arkavo_cwt::VerifyingKey::Ed25519(key))
            }
            Self::P256(keypair) => {
                let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(
                    &keypair.public_key().to_sec1_bytes(),
                )
                .expect("arkavo-crypto P-256 keys are valid SEC1 points");
                PermitVerifier(arkavo_cwt::VerifyingKey::P256(key))
            }
        }
    }

    /// COSE_Key form of the issuer's public half — the same key
    /// [`public_key`](Self::public_key) returns.
    ///
    /// It is not the `cnf` key. The `cnf` claim carries the presenter's
    /// COSE_Key, which [`mint`](crate::mint) takes as its own argument; a
    /// signer never supplies it.
    pub fn cose_key(&self) -> CoseKey {
        self.public_key().to_cose_key()
    }

    /// Sign `data`, producing the encoding required by RFC 8152: raw Ed25519
    /// signature bytes, or IEEE P1363 r||s for ES256.
    ///
    /// Both signatures a permit involves are made here: the COSE
    /// Sig_structure an issuer signs when minting, and the
    /// proof-of-possession digest a presenter signs when exercising the
    /// permit.
    ///
    /// Both branches produce the wire encoding directly, so this cannot fail
    /// and cannot fall back to a wrong-length signature that would only be
    /// discovered by a verifier later.
    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        match self {
            Self::Ed25519(keypair) => keypair.sign(data),
            Self::P256(keypair) => keypair.sign_p1363(data).to_vec(),
        }
    }
}

/// The permit's confirmation key. A thin wrapper so permit code keeps its
/// error type while the COSE key handling lives in `arkavo-cwt`.
#[derive(Clone, Debug)]
pub struct PermitVerifier(pub arkavo_cwt::VerifyingKey);

impl PermitVerifier {
    pub fn algorithm(&self) -> Algorithm {
        self.0.algorithm()
    }

    pub fn to_cose_key(&self) -> CoseKey {
        self.0.to_cose_key()
    }

    /// Recover a verification key from a COSE_Key, failing closed on any
    /// unexpected key type, curve, or parameter encoding.
    pub fn from_cose_key(key: &CoseKey) -> Result<Self, PermitError> {
        Ok(Self(arkavo_cwt::VerifyingKey::from_cose_key(key)?))
    }

    /// Build a verifier from raw public key bytes.
    ///
    /// 32 bytes for Ed25519, or 65-byte uncompressed SEC1 for P-256. Fails
    /// closed on any other length or malformed encoding.
    ///
    /// # Panics
    ///
    /// Never: the Ed25519 branch converts a slice whose length its own match
    /// arm has already fixed at 32.
    pub fn from_public_key_bytes(bytes: &[u8]) -> Result<Self, PermitError> {
        match bytes.len() {
            32 => {
                let raw: [u8; 32] = bytes.try_into().expect("length checked above");
                let key = ed25519_dalek::VerifyingKey::from_bytes(&raw)
                    .map_err(|e| PermitError::InvalidConfirmationKey(e.to_string()))?;
                Ok(Self(arkavo_cwt::VerifyingKey::Ed25519(key)))
            }
            65 => {
                let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(bytes)
                    .map_err(|e| PermitError::InvalidConfirmationKey(e.to_string()))?;
                Ok(Self(arkavo_cwt::VerifyingKey::P256(key)))
            }
            _ => Err(PermitError::InvalidConfirmationKey(
                "public key must be 32 raw Ed25519 bytes or 65 SEC1 P-256 bytes".to_string(),
            )),
        }
    }

    /// Verify a COSE signature value against a Sig_structure.
    pub fn verify(
        &self,
        algorithm: Algorithm,
        data: &[u8],
        signature: &[u8],
    ) -> Result<(), PermitError> {
        Ok(self.0.verify(algorithm, data, signature)?)
    }

    /// Raw public key bytes: 32 bytes for Ed25519, 65-byte SEC1 uncompressed
    /// for P-256.
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.0.public_key_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coset::CoseKeyBuilder;
    use coset::RegisteredLabel;
    use coset::iana::{EllipticCurve, KeyType};

    #[test]
    fn ed25519_cose_key_roundtrip() {
        let keypair = AgentKeypair::generate();
        let signer = PermitSigner::Ed25519(keypair);
        let cose_key = signer.cose_key();
        assert_eq!(cose_key.kty, RegisteredLabel::Assigned(KeyType::OKP));
        let verifier = PermitVerifier::from_cose_key(&cose_key).unwrap();
        assert_eq!(
            signer.public_key().public_key_bytes(),
            verifier.public_key_bytes()
        );
    }

    #[test]
    fn p256_cose_key_roundtrip() {
        let keypair = P256SigningKeypair::generate();
        let signer = PermitSigner::P256(keypair);
        let cose_key = signer.cose_key();
        assert_eq!(cose_key.kty, RegisteredLabel::Assigned(KeyType::EC2));
        let verifier = PermitVerifier::from_cose_key(&cose_key).unwrap();
        assert_eq!(
            signer.public_key().public_key_bytes(),
            verifier.public_key_bytes()
        );
    }

    #[test]
    fn ed25519_sign_verify_via_signer() {
        let signer = PermitSigner::Ed25519(AgentKeypair::generate());
        let verifier = signer.public_key();
        let data = b"sig structure bytes";
        let signature = signer.sign(data);
        assert_eq!(signature.len(), 64);
        assert!(verifier.verify(Algorithm::EdDSA, data, &signature).is_ok());
        assert!(
            verifier
                .verify(Algorithm::EdDSA, b"other", &signature)
                .is_err()
        );
    }

    #[test]
    fn p256_sign_produces_p1363_and_verifies() {
        let signer = PermitSigner::P256(P256SigningKeypair::generate());
        let verifier = signer.public_key();
        let data = b"sig structure bytes";
        let signature = signer.sign(data);
        assert_eq!(signature.len(), 64, "COSE ES256 requires raw r||s");
        assert!(verifier.verify(Algorithm::ES256, data, &signature).is_ok());
    }

    #[test]
    fn algorithm_mismatch_rejected() {
        let signer = PermitSigner::Ed25519(AgentKeypair::generate());
        let verifier = signer.public_key();
        let signature = signer.sign(b"data");
        assert!(matches!(
            verifier.verify(Algorithm::ES256, b"data", &signature),
            Err(PermitError::KeyAlgorithmMismatch)
        ));
    }

    #[test]
    fn wrong_curve_cose_key_rejected() {
        // An EC2 key on P-384 must not be accepted as an ES256 permit key.
        let key =
            CoseKeyBuilder::new_ec2_pub_key(EllipticCurve::P_384, vec![1; 48], vec![2; 48]).build();
        assert!(PermitVerifier::from_cose_key(&key).is_err());
    }

    #[test]
    fn ed25519_public_key_bytes_roundtrip_via_from_public_key_bytes() {
        let signer = PermitSigner::Ed25519(AgentKeypair::generate());
        let verifier = signer.public_key();
        let bytes = verifier.public_key_bytes();
        let recovered = PermitVerifier::from_public_key_bytes(&bytes).unwrap();
        assert_eq!(recovered.public_key_bytes(), bytes);
    }

    #[test]
    fn p256_public_key_bytes_roundtrip_via_from_public_key_bytes() {
        let signer = PermitSigner::P256(P256SigningKeypair::generate());
        let verifier = signer.public_key();
        let bytes = verifier.public_key_bytes();
        let recovered = PermitVerifier::from_public_key_bytes(&bytes).unwrap();
        assert_eq!(recovered.public_key_bytes(), bytes);
    }

    #[test]
    fn from_public_key_bytes_rejects_wrong_length() {
        let bytes = vec![0u8; 33];
        assert!(matches!(
            PermitVerifier::from_public_key_bytes(&bytes),
            Err(PermitError::InvalidConfirmationKey(_))
        ));
    }
}
