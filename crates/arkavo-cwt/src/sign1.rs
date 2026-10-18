//! COSE_Sign1 parsing shared by permit and bearer verification.

use crate::{CwtError, VerifyingKey};
use coset::iana::Algorithm;
use coset::{CborSerializable, CoseSign1, RegisteredLabelWithPrivate, TaggedCborSerializable};

/// CBOR tag 61 (CWT) as it appears on the wire.
pub const CWT_TAG_PREFIX: [u8; 2] = [0xd8, 0x3d];

/// Untrusted input larger than this is refused before any CBOR work.
pub const MAX_TOKEN_BYTES: usize = 16 * 1024;

pub struct ParsedSign1 {
    pub sign1: CoseSign1,
    /// The signature algorithm, EdDSA or ES256. RFC 9864's fully-specified
    /// Ed25519 and ESP256 are read as the algorithm they specialize.
    pub algorithm: Algorithm,
}

/// Parse a CWT-shaped COSE_Sign1. The tag-61 prefix is optional and the
/// COSE_Sign1 may be tagged (18) or bare: authnz-rs emits bare, permits
/// emit tagged, and both must verify through the same code.
///
/// Two bounds are applied to untrusted input before any CBOR decoder sees
/// it: [`MAX_TOKEN_BYTES`] on the length, and
/// [`crate::depth::MAX_NESTING_DEPTH`] on the nesting — of the token and of
/// the CBOR inside the two byte strings a decoder walks in their own right,
/// the protected header and the payload. Either of those two encoded with an
/// indefinite length is refused rather than walked.
pub fn parse(bytes: &[u8]) -> Result<ParsedSign1, CwtError> {
    if bytes.len() > MAX_TOKEN_BYTES {
        return Err(CwtError::Cose("token exceeds maximum size".into()));
    }
    let body = bytes.strip_prefix(&CWT_TAG_PREFIX[..]).unwrap_or(bytes);
    crate::depth::check(body)?;
    let sign1 = CoseSign1::from_tagged_slice(body)
        .or_else(|_| CoseSign1::from_slice(body))
        .map_err(|e| CwtError::Cose(e.to_string()))?;
    let algorithm = match &sign1.protected.header.alg {
        Some(RegisteredLabelWithPrivate::Assigned(alg))
            if matches!(
                crate::key::polymorphic(*alg),
                Algorithm::EdDSA | Algorithm::ES256
            ) =>
        {
            crate::key::polymorphic(*alg)
        }
        Some(other) => return Err(CwtError::UnsupportedAlgorithm(format!("{other:?}"))),
        None => return Err(CwtError::UnsupportedAlgorithm("none".into())),
    };
    Ok(ParsedSign1 { sign1, algorithm })
}

impl ParsedSign1 {
    /// The key identifier from the **protected** header only.
    ///
    /// A `kid` in the unprotected header is deliberately ignored: that bucket
    /// is outside the signature, so honouring it would let anyone redirect a
    /// token at a different published key.
    pub fn kid(&self) -> &[u8] {
        &self.sign1.protected.header.key_id
    }

    pub fn payload(&self) -> Result<&[u8], CwtError> {
        self.sign1
            .payload
            .as_deref()
            .ok_or_else(|| CwtError::Cose("payload is detached".into()))
    }

    pub fn verify(&self, key: &VerifyingKey) -> Result<(), CwtError> {
        self.sign1.verify_signature(b"", |signature, data| {
            key.verify(self.algorithm, data, signature)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coset::{CborSerializable, CoseSign1Builder, HeaderBuilder, TaggedCborSerializable};
    use ed25519_dalek::Signer as _;

    fn signed(tagged: bool, prefix: bool) -> (Vec<u8>, VerifyingKey) {
        signed_payload(b"payload".to_vec(), tagged, prefix)
    }

    fn signed_payload(payload: Vec<u8>, tagged: bool, prefix: bool) -> (Vec<u8>, VerifyingKey) {
        let signing = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        let protected = HeaderBuilder::new()
            .algorithm(coset::iana::Algorithm::EdDSA)
            .key_id(b"k1".to_vec())
            .build();
        let sign1 = CoseSign1Builder::new()
            .protected(protected)
            .payload(payload)
            .create_signature(b"", |data| signing.sign(data).to_bytes().to_vec())
            .build();
        let mut bytes = if prefix {
            CWT_TAG_PREFIX.to_vec()
        } else {
            Vec::new()
        };
        if tagged {
            bytes.extend(sign1.to_tagged_vec().unwrap());
        } else {
            bytes.extend(sign1.to_vec().unwrap());
        }
        (bytes, VerifyingKey::Ed25519(signing.verifying_key()))
    }

    fn signed_with(algorithm: coset::iana::Algorithm, key: &VerifyingKeyMaker) -> Vec<u8> {
        let protected = HeaderBuilder::new()
            .algorithm(algorithm)
            .key_id(b"k1".to_vec())
            .build();
        CoseSign1Builder::new()
            .protected(protected)
            .payload(b"payload".to_vec())
            .create_signature(b"", |data| key.sign(data))
            .build()
            .to_tagged_vec()
            .unwrap()
    }

    enum VerifyingKeyMaker {
        Ed25519(ed25519_dalek::SigningKey),
        P256(p256::ecdsa::SigningKey),
    }

    impl VerifyingKeyMaker {
        fn sign(&self, data: &[u8]) -> Vec<u8> {
            match self {
                Self::Ed25519(key) => key.sign(data).to_bytes().to_vec(),
                Self::P256(key) => {
                    let signature: p256::ecdsa::Signature = key.sign(data);
                    signature.to_bytes().to_vec()
                }
            }
        }

        fn verifying(&self) -> VerifyingKey {
            match self {
                Self::Ed25519(key) => VerifyingKey::Ed25519(key.verifying_key()),
                Self::P256(key) => VerifyingKey::P256(*key.verifying_key()),
            }
        }
    }

    /// RFC 9864's fully-specified identifiers verify as the algorithms they
    /// specialize.
    #[arkavo_test_macros::spec("ACWT-004")]
    #[test]
    fn fully_specified_algorithms_verify_as_the_ones_they_specialize() {
        use coset::iana::Algorithm;
        let ed25519 =
            VerifyingKeyMaker::Ed25519(ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng));
        let p256 = VerifyingKeyMaker::P256(p256::ecdsa::SigningKey::random(&mut rand::rngs::OsRng));
        for (algorithm, key, read_as) in [
            (Algorithm::Ed25519, &ed25519, Algorithm::EdDSA),
            (Algorithm::EdDSA, &ed25519, Algorithm::EdDSA),
            (Algorithm::ESP256, &p256, Algorithm::ES256),
            (Algorithm::ES256, &p256, Algorithm::ES256),
        ] {
            let parsed = parse(&signed_with(algorithm, key)).unwrap();
            assert_eq!(parsed.algorithm, read_as, "{algorithm:?}");
            parsed.verify(&key.verifying()).unwrap();
        }
    }

    /// A fully-specified identifier names its curve: ESP256 is not checked
    /// with an Ed25519 key, nor Ed25519 with a P-256 one.
    #[arkavo_test_macros::spec("ACWT-004")]
    #[test]
    fn a_fully_specified_algorithm_does_not_verify_with_another_curves_key() {
        use coset::iana::Algorithm;
        let ed25519 =
            VerifyingKeyMaker::Ed25519(ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng));
        let p256 = VerifyingKeyMaker::P256(p256::ecdsa::SigningKey::random(&mut rand::rngs::OsRng));
        let esp256 = parse(&signed_with(Algorithm::ESP256, &p256)).unwrap();
        assert!(matches!(
            esp256.verify(&ed25519.verifying()),
            Err(CwtError::KeyAlgorithmMismatch)
        ));
        let eddsa = parse(&signed_with(Algorithm::Ed25519, &ed25519)).unwrap();
        assert!(matches!(
            eddsa.verify(&p256.verifying()),
            Err(CwtError::KeyAlgorithmMismatch)
        ));
    }

    #[test]
    fn parses_all_four_wire_shapes() {
        for (tagged, prefix) in [(true, true), (true, false), (false, true), (false, false)] {
            let (bytes, key) = signed(tagged, prefix);
            let parsed = parse(&bytes).unwrap();
            assert_eq!(parsed.algorithm, coset::iana::Algorithm::EdDSA);
            assert_eq!(parsed.kid(), b"k1");
            assert_eq!(parsed.payload().unwrap(), b"payload");
            parsed.verify(&key).unwrap();
        }
    }

    #[test]
    fn rejects_oversized_input_before_parsing() {
        // A well-formed, correctly signed token that is merely too large:
        // nothing but the size gate can refuse it, so the assertion pins that
        // gate instead of passing on malformed CBOR.
        let (big, _) = signed_payload(vec![b'a'; MAX_TOKEN_BYTES], true, true);
        assert!(big.len() > MAX_TOKEN_BYTES);
        assert!(matches!(
            parse(&big),
            Err(CwtError::Cose(msg)) if msg.contains("maximum size")
        ));
        // The same shape under the cap parses and verifies, so the refusal
        // above is about size and nothing else.
        let (small, key) = signed_payload(vec![b'a'; 64], true, true);
        let parsed = parse(&small).unwrap();
        assert_eq!(parsed.payload().unwrap().len(), 64);
        parsed.verify(&key).unwrap();
    }

    /// A token whose CBOR nests thousands of levels deep sits well under the
    /// 16 KiB cap — one byte buys one level — and reaches ciborium's
    /// recursive decoder before any signature is checked. The depth bound is
    /// what refuses it.
    #[test]
    fn rejects_deeply_nested_cbor_under_the_size_cap() {
        let mut deep = vec![0x81u8; 200];
        deep.push(0x00);
        assert!(deep.len() < MAX_TOKEN_BYTES);
        assert!(matches!(
            parse(&deep),
            Err(CwtError::Cose(message)) if message.contains("nesting depth")
        ));

        // The same depth hidden in the payload byte string, where a decoder
        // walks it just as recursively.
        let (nested_payload, _) = signed_payload(deep, true, true);
        assert!(matches!(
            parse(&nested_payload),
            Err(CwtError::Cose(message)) if message.contains("nesting depth")
        ));

        // An ordinary token is unaffected: it still parses and verifies.
        let (bytes, key) = signed(true, true);
        let parsed = parse(&bytes).expect("a normal token still parses");
        parsed.verify(&key).expect("and still verifies");
    }

    #[test]
    fn rejects_missing_alg() {
        let sign1 = CoseSign1Builder::new().payload(b"p".to_vec()).build();
        let bytes = sign1.to_vec().unwrap();
        assert!(matches!(
            parse(&bytes),
            Err(CwtError::UnsupportedAlgorithm(_))
        ));
    }

    #[test]
    fn wrong_key_fails_verification() {
        let (bytes, _) = signed(true, true);
        let other = VerifyingKey::Ed25519(
            ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng).verifying_key(),
        );
        assert!(matches!(
            parse(&bytes).unwrap().verify(&other),
            Err(CwtError::BadSignature)
        ));
    }
}
