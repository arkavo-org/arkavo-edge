//! Signing and verification keys for permits, reusing `arkavo-crypto` key
//! types (Ed25519 primary, P-256/ES256 supported) and encoding them as
//! COSE_Key confirmation keys per RFC 8747.

use crate::error::PermitError;
use arkavo_crypto::{AgentKeypair, AgentPublicKey, P256SigningKeypair, P256VerifyingKey};
use ciborium::value::{Integer, Value};
use coset::iana::{Algorithm, Ec2KeyParameter, EllipticCurve, EnumI64, KeyType, OkpKeyParameter};
use coset::{CoseKey, CoseKeyBuilder, Label, RegisteredLabel};

/// A permit signing key. Determines the COSE algorithm in the protected
/// header and the COSE_Key placed in the `cnf` claim.
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

    pub fn public_key(&self) -> PermitVerifier {
        match self {
            Self::Ed25519(keypair) => PermitVerifier::Ed25519(keypair.public_key()),
            Self::P256(keypair) => PermitVerifier::P256(keypair.public_key()),
        }
    }

    /// COSE_Key for the public half, suitable for the `cnf` claim.
    pub fn cose_key(&self) -> CoseKey {
        self.public_key().to_cose_key()
    }

    /// Sign a COSE Sig_structure, producing the encoding required by RFC
    /// 8152: raw Ed25519 signature bytes, or IEEE P1363 r||s for ES256.
    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        match self {
            Self::Ed25519(keypair) => keypair.sign(data),
            Self::P256(keypair) => {
                let der = keypair.sign(data);
                // Infallible for signatures produced by p256::ecdsa.
                der_to_p1363(&der).unwrap_or(der)
            }
        }
    }
}

/// A permit verification key, recovered from the `cnf` claim or supplied
/// out of band.
#[derive(Clone)]
pub enum PermitVerifier {
    Ed25519(AgentPublicKey),
    P256(P256VerifyingKey),
}

impl PermitVerifier {
    pub fn algorithm(&self) -> Algorithm {
        match self {
            Self::Ed25519(_) => Algorithm::EdDSA,
            Self::P256(_) => Algorithm::ES256,
        }
    }

    pub fn to_cose_key(&self) -> CoseKey {
        match self {
            Self::Ed25519(key) => CoseKeyBuilder::new_okp_key()
                .algorithm(Algorithm::EdDSA)
                .param(
                    OkpKeyParameter::Crv.to_i64(),
                    int_value(EllipticCurve::Ed25519.to_i64()),
                )
                .param(OkpKeyParameter::X.to_i64(), Value::Bytes(key.to_bytes()))
                .build(),
            Self::P256(key) => {
                let sec1 = key.to_sec1_bytes();
                CoseKeyBuilder::new_ec2_pub_key(
                    EllipticCurve::P_256,
                    sec1[1..33].to_vec(),
                    sec1[33..65].to_vec(),
                )
                .algorithm(Algorithm::ES256)
                .build()
            }
        }
    }

    /// Recover a verification key from a COSE_Key, failing closed on any
    /// unexpected key type, curve, or parameter encoding.
    pub fn from_cose_key(key: &CoseKey) -> Result<Self, PermitError> {
        match key.kty {
            RegisteredLabel::Assigned(KeyType::OKP) => Self::from_okp(key),
            RegisteredLabel::Assigned(KeyType::EC2) => Self::from_ec2(key),
            _ => Err(PermitError::InvalidConfirmationKey(
                "kty must be OKP (Ed25519) or EC2 (P-256)".to_string(),
            )),
        }
    }

    fn from_okp(key: &CoseKey) -> Result<Self, PermitError> {
        expect_curve(key, EllipticCurve::Ed25519)?;
        let x = param_bytes(key, OkpKeyParameter::X.to_i64())?;
        let public = AgentPublicKey::from_bytes(&x)
            .map_err(|e| PermitError::InvalidConfirmationKey(e.to_string()))?;
        Ok(Self::Ed25519(public))
    }

    fn from_ec2(key: &CoseKey) -> Result<Self, PermitError> {
        expect_curve(key, EllipticCurve::P_256)?;
        let x = param_bytes(key, Ec2KeyParameter::X.to_i64())?;
        let y = param_bytes(key, Ec2KeyParameter::Y.to_i64())?;
        if x.len() != 32 || y.len() != 32 {
            return Err(PermitError::InvalidConfirmationKey(
                "P-256 coordinates must be 32 bytes".to_string(),
            ));
        }
        let mut sec1 = Vec::with_capacity(65);
        sec1.push(0x04);
        sec1.extend_from_slice(&x);
        sec1.extend_from_slice(&y);
        let public = P256VerifyingKey::from_sec1_bytes(&sec1)
            .map_err(|e| PermitError::InvalidConfirmationKey(e.to_string()))?;
        Ok(Self::P256(public))
    }

    /// Verify a COSE signature value against a Sig_structure.
    pub fn verify(
        &self,
        algorithm: Algorithm,
        data: &[u8],
        signature: &[u8],
    ) -> Result<(), PermitError> {
        if algorithm != self.algorithm() {
            return Err(PermitError::KeyAlgorithmMismatch);
        }
        match self {
            Self::Ed25519(key) => key
                .verify(data, signature)
                .map_err(|_| PermitError::InvalidSignature),
            Self::P256(key) => key
                .verify(data, signature)
                .map_err(|_| PermitError::InvalidSignature),
        }
    }

    /// Raw public key bytes: 32 bytes for Ed25519, 65-byte SEC1 uncompressed
    /// for P-256.
    pub fn public_key_bytes(&self) -> Vec<u8> {
        match self {
            Self::Ed25519(key) => key.to_bytes(),
            Self::P256(key) => key.to_sec1_bytes(),
        }
    }
}

fn int_value(value: i64) -> Value {
    Value::Integer(Integer::from(value))
}

fn param_bytes(key: &CoseKey, label: i64) -> Result<Vec<u8>, PermitError> {
    let wanted = Label::Int(label);
    match key.params.iter().find(|(l, _)| *l == wanted) {
        Some((_, Value::Bytes(bytes))) => Ok(bytes.clone()),
        Some(_) => Err(PermitError::InvalidConfirmationKey(format!(
            "parameter {label} is not a bstr"
        ))),
        None => Err(PermitError::InvalidConfirmationKey(format!(
            "missing parameter {label}"
        ))),
    }
}

fn expect_curve(key: &CoseKey, curve: EllipticCurve) -> Result<(), PermitError> {
    let wanted = Integer::from(curve.to_i64());
    let found = key
        .params
        .iter()
        .find_map(|(label, value)| match (label, value) {
            (Label::Int(l), Value::Integer(v)) if *l == -1 => Some(*v),
            _ => None,
        });
    match found {
        Some(v) if v == wanted => Ok(()),
        _ => Err(PermitError::InvalidConfirmationKey(format!(
            "unexpected curve, want {:?}",
            curve.to_i64()
        ))),
    }
}

/// Convert a DER-encoded ECDSA signature to the IEEE P1363 fixed-size
/// r||s form required by COSE ES256 (RFC 8152 section 8.1).
fn der_to_p1363(der: &[u8]) -> Result<Vec<u8>, PermitError> {
    fn read_len(bytes: &[u8], pos: &mut usize) -> Result<usize, PermitError> {
        let first = *bytes
            .get(*pos)
            .ok_or_else(|| PermitError::Cose("truncated DER signature".to_string()))?;
        *pos += 1;
        if first & 0x80 == 0 {
            return Ok(first as usize);
        }
        let count = (first & 0x7f) as usize;
        if count == 0 || count > 2 {
            return Err(PermitError::Cose("invalid DER length".to_string()));
        }
        let mut len = 0usize;
        for _ in 0..count {
            let byte = *bytes
                .get(*pos)
                .ok_or_else(|| PermitError::Cose("truncated DER length".to_string()))?;
            *pos += 1;
            len = (len << 8) | byte as usize;
        }
        Ok(len)
    }

    let mut pos = 0usize;
    let bad = || PermitError::Cose("malformed DER ECDSA signature".to_string());
    if der.first() != Some(&0x30) {
        return Err(bad());
    }
    pos += 1;
    let seq_len = read_len(der, &mut pos)?;
    if pos + seq_len != der.len() {
        return Err(bad());
    }
    let mut out = Vec::with_capacity(64);
    for _ in 0..2 {
        if der.get(pos) != Some(&0x02) {
            return Err(bad());
        }
        pos += 1;
        let int_len = read_len(der, &mut pos)?;
        let bytes = der.get(pos..pos + int_len).ok_or_else(bad)?;
        pos += int_len;
        // Strip the sign-padding zero, then left-pad to 32 bytes.
        let significant = bytes
            .iter()
            .position(|b| *b != 0)
            .map(|i| &bytes[i..])
            .unwrap_or(&[][..]);
        if significant.len() > 32 {
            return Err(bad());
        }
        out.resize(out.len() + (32 - significant.len()), 0);
        out.extend_from_slice(significant);
    }
    if pos != der.len() {
        return Err(bad());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn der_to_p1363_converts_and_rejects_garbage() {
        let keypair = P256SigningKeypair::generate();
        let der = keypair.sign(b"message");
        assert_eq!(der[0], 0x30);
        let raw = der_to_p1363(&der).unwrap();
        assert_eq!(raw.len(), 64);
        keypair
            .public_key()
            .verify(b"message", &raw)
            .expect("raw signature must verify");

        assert!(der_to_p1363(&[]).is_err());
        assert!(der_to_p1363(&[0x30]).is_err());
        assert!(der_to_p1363(&[0x31, 0x00]).is_err());
        let mut truncated = der.clone();
        truncated.truncate(der.len() - 1);
        assert!(der_to_p1363(&truncated).is_err());
    }
}
