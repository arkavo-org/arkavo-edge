//! One verifying-key type for every CWT the edge checks. Permits carry the
//! key inline in `cnf`; bearer tokens look it up by `kid`. Both end here.

use crate::CwtError;
use ciborium::Value;
use coset::iana::{
    Algorithm, Ec2KeyParameter, EllipticCurve, EnumI64, KeyOperation, KeyType, OkpKeyParameter,
};
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

    /// Recover a verification key from a COSE_Key.
    ///
    /// `alg` and `key_ops` are honoured when present (RFC 8152 section 7):
    /// a key that declares an algorithm must declare the one its type signs
    /// with, and a key that enumerates its operations must include `verify`.
    /// Both are optional in COSE, and a key that states neither is used as
    /// its `kty` and curve describe it.
    pub fn from_cose_key(key: &CoseKey) -> Result<Self, CwtError> {
        expect_key_ops(key)?;
        match key.kty {
            RegisteredLabel::Assigned(KeyType::OKP) => {
                expect_curve(key, OkpKeyParameter::Crv.to_i64(), EllipticCurve::Ed25519)?;
                expect_algorithm(key, Algorithm::EdDSA)?;
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
                expect_algorithm(key, Algorithm::ES256)?;
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

/// A COSE key may restrict itself to one algorithm (RFC 8152 section 7,
/// label 3). When it does, it must be the algorithm its key type signs with:
/// honouring a key that names something else would let a publisher's stated
/// restriction be ignored.
fn expect_algorithm(key: &CoseKey, expected: Algorithm) -> Result<(), CwtError> {
    match &key.alg {
        None => Ok(()),
        Some(coset::RegisteredLabelWithPrivate::Assigned(actual)) if *actual == expected => Ok(()),
        Some(actual) => Err(CwtError::Key(format!(
            "COSE key declares alg {actual:?}, which is not {expected:?}"
        ))),
    }
}

/// A COSE key may enumerate the operations it is for (label 4). A key that
/// does must include `verify`; one that does not is unrestricted.
fn expect_key_ops(key: &CoseKey) -> Result<(), CwtError> {
    if key.key_ops.is_empty()
        || key
            .key_ops
            .contains(&RegisteredLabel::Assigned(KeyOperation::Verify))
    {
        Ok(())
    } else {
        Err(CwtError::Key(
            "COSE key's key_ops does not include verify".into(),
        ))
    }
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

    /// RFC 8152 section 7: `alg` restricts the key to one algorithm. A key
    /// published as ES256 must not be accepted as an Ed25519 verifier just
    /// because its `kty` and curve line up.
    #[test]
    fn declared_algorithm_must_match_the_key_type() {
        let signing = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        let mut cose = VerifyingKey::Ed25519(signing.verifying_key()).to_cose_key();
        cose.alg = Some(coset::RegisteredLabelWithPrivate::Assigned(
            Algorithm::ES256,
        ));
        assert!(matches!(
            VerifyingKey::from_cose_key(&cose),
            Err(CwtError::Key(message)) if message.contains("alg")
        ));

        // A key that declares nothing is used as its type describes it.
        cose.alg = None;
        assert!(VerifyingKey::from_cose_key(&cose).is_ok());
    }

    /// `key_ops` says what the key may be used for. One published for signing
    /// only is not a verification key, whatever its coordinates say.
    #[test]
    fn key_ops_must_include_verify_when_present() {
        let signing = p256::ecdsa::SigningKey::random(&mut rand::rngs::OsRng);
        let mut cose = VerifyingKey::P256(*signing.verifying_key()).to_cose_key();
        cose.key_ops = [RegisteredLabel::Assigned(KeyOperation::Sign)]
            .into_iter()
            .collect();
        assert!(matches!(
            VerifyingKey::from_cose_key(&cose),
            Err(CwtError::Key(message)) if message.contains("key_ops")
        ));

        cose.key_ops = [
            RegisteredLabel::Assigned(KeyOperation::Sign),
            RegisteredLabel::Assigned(KeyOperation::Verify),
        ]
        .into_iter()
        .collect();
        assert!(VerifyingKey::from_cose_key(&cose).is_ok());
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
