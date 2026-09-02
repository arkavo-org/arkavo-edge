//! One verifying-key type for every CWT the edge checks. Permits carry the
//! key inline in `cnf`; bearer tokens look it up by `kid`. Both end here.

use crate::CwtError;
use ciborium::Value;
use coset::iana::{Algorithm, Ec2KeyParameter, EllipticCurve, EnumI64, KeyType, OkpKeyParameter};
use coset::{CoseKey, CoseKeyBuilder, Label, RegisteredLabel};
use p256::ecdsa::signature::Verifier as _;

#[derive(Clone, Debug)]
pub enum VerifyingKey {
    Ed25519(ed25519_dalek::VerifyingKey),
    P256(p256::ecdsa::VerifyingKey),
}

impl VerifyingKey {
    pub fn algorithm(&self) -> Algorithm {
        match self {
            Self::Ed25519(_) => Algorithm::EdDSA,
            Self::P256(_) => Algorithm::ES256,
        }
    }

    pub fn from_cose_key(key: &CoseKey) -> Result<Self, CwtError> {
        match key.kty {
            RegisteredLabel::Assigned(KeyType::OKP) => {
                expect_curve(key, OkpKeyParameter::Crv.to_i64(), EllipticCurve::Ed25519)?;
                let x = bytes_param(key, OkpKeyParameter::X.to_i64(), "x")?;
                let raw: [u8; 32] = x
                    .try_into()
                    .map_err(|_| CwtError::Key("Ed25519 x must be 32 bytes".into()))?;
                ed25519_dalek::VerifyingKey::from_bytes(&raw)
                    .map(Self::Ed25519)
                    .map_err(|e| CwtError::Key(e.to_string()))
            }
            RegisteredLabel::Assigned(KeyType::EC2) => {
                expect_curve(key, Ec2KeyParameter::Crv.to_i64(), EllipticCurve::P_256)?;
                let x = bytes_param(key, Ec2KeyParameter::X.to_i64(), "x")?;
                let y = bytes_param(key, Ec2KeyParameter::Y.to_i64(), "y")?;
                if x.len() != 32 || y.len() != 32 {
                    return Err(CwtError::Key("P-256 coordinates must be 32 bytes".into()));
                }
                let point = p256::EncodedPoint::from_affine_coordinates(
                    p256::FieldBytes::from_slice(x),
                    p256::FieldBytes::from_slice(y),
                    false,
                );
                p256::ecdsa::VerifyingKey::from_encoded_point(&point)
                    .map(Self::P256)
                    .map_err(|e| CwtError::Key(e.to_string()))
            }
            _ => Err(CwtError::Key("key type is neither OKP nor EC2".into())),
        }
    }

    pub fn to_cose_key(&self) -> CoseKey {
        match self {
            Self::Ed25519(key) => CoseKeyBuilder::new_okp_key()
                .param(
                    OkpKeyParameter::Crv.to_i64(),
                    Value::from(EllipticCurve::Ed25519.to_i64()),
                )
                .param(
                    OkpKeyParameter::X.to_i64(),
                    Value::Bytes(key.to_bytes().to_vec()),
                )
                .algorithm(Algorithm::EdDSA)
                .build(),
            Self::P256(key) => {
                let point = key.to_encoded_point(false);
                let x = point.x().map(|c| c.to_vec()).unwrap_or_default();
                let y = point.y().map(|c| c.to_vec()).unwrap_or_default();
                CoseKeyBuilder::new_ec2_pub_key(EllipticCurve::P_256, x, y)
                    .algorithm(Algorithm::ES256)
                    .build()
            }
        }
    }

    pub fn verify(
        &self,
        algorithm: Algorithm,
        data: &[u8],
        signature: &[u8],
    ) -> Result<(), CwtError> {
        if algorithm != self.algorithm() {
            return Err(CwtError::KeyAlgorithmMismatch);
        }
        match self {
            Self::Ed25519(key) => {
                let sig = ed25519_dalek::Signature::from_slice(signature)
                    .map_err(|_| CwtError::BadSignature)?;
                key.verify_strict(data, &sig)
                    .map_err(|_| CwtError::BadSignature)
            }
            Self::P256(key) => {
                let sig = p256::ecdsa::Signature::from_slice(signature)
                    .map_err(|_| CwtError::BadSignature)?;
                key.verify(data, &sig).map_err(|_| CwtError::BadSignature)
            }
        }
    }

    /// 32 raw bytes for Ed25519, 65-byte uncompressed SEC1 for P-256.
    pub fn public_key_bytes(&self) -> Vec<u8> {
        match self {
            Self::Ed25519(key) => key.to_bytes().to_vec(),
            Self::P256(key) => key.to_encoded_point(false).as_bytes().to_vec(),
        }
    }
}

fn param(key: &CoseKey, label: i64) -> Option<&Value> {
    key.params
        .iter()
        .find(|(candidate, _)| *candidate == Label::Int(label))
        .map(|(_, value)| value)
}

fn bytes_param<'a>(key: &'a CoseKey, label: i64, name: &str) -> Result<&'a [u8], CwtError> {
    param(key, label)
        .and_then(Value::as_bytes)
        .map(Vec::as_slice)
        .ok_or_else(|| CwtError::Key(format!("COSE key is missing its {name} coordinate")))
}

fn expect_curve(key: &CoseKey, label: i64, curve: EllipticCurve) -> Result<(), CwtError> {
    let actual = param(key, label).and_then(Value::as_integer);
    if actual == Some(curve.to_i64().into()) {
        Ok(())
    } else {
        Err(CwtError::Key(format!("unexpected curve {actual:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coset::iana::Algorithm;
    use ed25519_dalek::Signer as _;

    #[test]
    fn ed25519_round_trips_through_cose_key() {
        let signing = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        let key = VerifyingKey::Ed25519(signing.verifying_key());
        let back = VerifyingKey::from_cose_key(&key.to_cose_key()).unwrap();
        assert_eq!(back.public_key_bytes(), key.public_key_bytes());
        let sig = signing.sign(b"data");
        back.verify(Algorithm::EdDSA, b"data", &sig.to_bytes())
            .unwrap();
    }

    #[test]
    fn p256_round_trips_and_rejects_wrong_algorithm() {
        let signing = p256::ecdsa::SigningKey::random(&mut rand::rngs::OsRng);
        let key = VerifyingKey::P256(*signing.verifying_key());
        let back = VerifyingKey::from_cose_key(&key.to_cose_key()).unwrap();
        let sig: p256::ecdsa::Signature = signing.sign(b"data");
        back.verify(Algorithm::ES256, b"data", &sig.to_bytes())
            .unwrap();
        assert!(matches!(
            back.verify(Algorithm::EdDSA, b"data", &sig.to_bytes()),
            Err(CwtError::KeyAlgorithmMismatch)
        ));
    }

    #[test]
    fn tampered_signature_is_bad_signature() {
        let signing = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        let key = VerifyingKey::Ed25519(signing.verifying_key());
        let mut sig = signing.sign(b"data").to_bytes();
        sig[0] ^= 0x01;
        assert!(matches!(
            key.verify(Algorithm::EdDSA, b"data", &sig),
            Err(CwtError::BadSignature)
        ));
    }

    #[test]
    fn short_p256_coordinate_is_rejected() {
        let mut cose = VerifyingKey::P256(
            *p256::ecdsa::SigningKey::random(&mut rand::rngs::OsRng).verifying_key(),
        )
        .to_cose_key();
        for (label, value) in cose.params.iter_mut() {
            if *label == coset::Label::Int(coset::iana::Ec2KeyParameter::X as i64)
                && let ciborium::Value::Bytes(bytes) = value
            {
                bytes.truncate(31);
            }
        }
        assert!(matches!(
            VerifyingKey::from_cose_key(&cose),
            Err(CwtError::Key(_))
        ));
    }
}
