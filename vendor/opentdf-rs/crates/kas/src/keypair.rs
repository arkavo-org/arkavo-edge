//! KAS keypair management for EC and RSA keys
//!
//! # EC Public Key Format
//!
//! EC public keys are encoded using SEC1 uncompressed format (65 bytes for P-256)
//! wrapped in a PEM block with "PUBLIC KEY" label. This format stores the raw
//! elliptic curve point coordinates.
//!
//! For external interoperability with systems expecting SPKI (SubjectPublicKeyInfo)
//! format, the raw SEC1 bytes can be extracted and re-encoded as needed. The SEC1
//! format is used here for simplicity and direct compatibility with the p256 crate.

use crate::error::KasServerError;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey as P256PublicKey, SecretKey as P256SecretKey};

/// EC P-256 keypair for NanoTDF operations
#[derive(Clone)]
pub struct KasEcKeypair {
    private_key: P256SecretKey,
    public_key: P256PublicKey,
    public_key_pem: String,
}

impl KasEcKeypair {
    /// Create from existing private key bytes (32 bytes)
    pub fn from_private_key_bytes(bytes: &[u8]) -> Result<Self, KasServerError> {
        let private_key = P256SecretKey::from_slice(bytes)
            .map_err(|e| KasServerError::InvalidPrivateKey(e.to_string()))?;
        let public_key = private_key.public_key();
        let public_key_pem = Self::encode_public_key_pem(&public_key)?;

        Ok(Self {
            private_key,
            public_key,
            public_key_pem,
        })
    }

    /// Generate a new random keypair
    pub fn generate() -> Result<Self, KasServerError> {
        use rand::rngs::OsRng;
        let private_key = P256SecretKey::random(&mut OsRng);
        let public_key = private_key.public_key();
        let public_key_pem = Self::encode_public_key_pem(&public_key)?;

        Ok(Self {
            private_key,
            public_key,
            public_key_pem,
        })
    }

    /// Get the private key
    pub fn private_key(&self) -> &P256SecretKey {
        &self.private_key
    }

    /// Get the public key
    pub fn public_key(&self) -> &P256PublicKey {
        &self.public_key
    }

    /// Get the public key in PEM format
    pub fn public_key_pem(&self) -> &str {
        &self.public_key_pem
    }

    /// Encode public key to PEM format
    fn encode_public_key_pem(public_key: &P256PublicKey) -> Result<String, KasServerError> {
        let encoded_point = public_key.to_encoded_point(false);
        let sec1_bytes = encoded_point.as_bytes();
        let pem_encoded = pem::Pem::new("PUBLIC KEY", sec1_bytes.to_vec());
        Ok(pem::encode(&pem_encoded))
    }

    /// Parse PEM-encoded public key
    pub fn parse_public_key_pem(pem_str: &str) -> Result<P256PublicKey, KasServerError> {
        let pem_parsed =
            pem::parse(pem_str).map_err(|e| KasServerError::InvalidPublicKey(e.to_string()))?;
        P256PublicKey::from_sec1_bytes(pem_parsed.contents())
            .map_err(|e| KasServerError::InvalidPublicKey(e.to_string()))
    }
}

/// RSA keypair for Standard TDF operations
#[cfg(feature = "rsa")]
pub struct KasRsaKeypair {
    private_key: rsa::RsaPrivateKey,
    public_key_pem: String,
}

#[cfg(feature = "rsa")]
impl KasRsaKeypair {
    /// Create from existing private key in PKCS#8 PEM format
    pub fn from_pkcs8_pem(pem_str: &str) -> Result<Self, KasServerError> {
        use pkcs8::DecodePrivateKey;
        let private_key = rsa::RsaPrivateKey::from_pkcs8_pem(pem_str)
            .map_err(|e| KasServerError::InvalidPrivateKey(e.to_string()))?;
        let public_key_pem = Self::encode_public_key_pem(&private_key)?;

        Ok(Self {
            private_key,
            public_key_pem,
        })
    }

    /// Generate a new random 2048-bit RSA keypair
    pub fn generate() -> Result<Self, KasServerError> {
        use rand::rngs::OsRng;
        let private_key = rsa::RsaPrivateKey::new(&mut OsRng, 2048)
            .map_err(|e| KasServerError::InvalidPrivateKey(e.to_string()))?;
        let public_key_pem = Self::encode_public_key_pem(&private_key)?;

        Ok(Self {
            private_key,
            public_key_pem,
        })
    }

    /// Get the private key
    pub fn private_key(&self) -> &rsa::RsaPrivateKey {
        &self.private_key
    }

    /// Get the public key in PEM format
    pub fn public_key_pem(&self) -> &str {
        &self.public_key_pem
    }

    /// Encode public key to PEM format
    fn encode_public_key_pem(private_key: &rsa::RsaPrivateKey) -> Result<String, KasServerError> {
        use pkcs8::EncodePublicKey;
        use rsa::RsaPublicKey;

        let public_key = RsaPublicKey::from(private_key);
        public_key
            .to_public_key_pem(pkcs8::LineEnding::LF)
            .map_err(|e| KasServerError::InvalidPublicKey(e.to_string()))
    }
}

/// Combined KAS keypair supporting both EC and RSA
pub struct KasKeypair {
    ec: KasEcKeypair,
    #[cfg(feature = "rsa")]
    rsa: Option<KasRsaKeypair>,
}

impl KasKeypair {
    /// Create with only EC keypair
    pub fn ec_only(ec: KasEcKeypair) -> Self {
        Self {
            ec,
            #[cfg(feature = "rsa")]
            rsa: None,
        }
    }

    /// Create with EC and RSA keypairs
    #[cfg(feature = "rsa")]
    pub fn new(ec: KasEcKeypair, rsa: Option<KasRsaKeypair>) -> Self {
        Self { ec, rsa }
    }

    /// Get the EC keypair
    pub fn ec(&self) -> &KasEcKeypair {
        &self.ec
    }

    /// Get the RSA keypair
    #[cfg(feature = "rsa")]
    pub fn rsa(&self) -> Option<&KasRsaKeypair> {
        self.rsa.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ec_keypair_generate() {
        let keypair = KasEcKeypair::generate().unwrap();
        assert!(!keypair.public_key_pem().is_empty());
        assert!(keypair.public_key_pem().contains("PUBLIC KEY"));
    }

    #[test]
    fn test_ec_keypair_from_bytes() {
        // Generate a keypair first
        let keypair1 = KasEcKeypair::generate().unwrap();
        let private_bytes = keypair1.private_key().to_bytes();

        // Recreate from bytes
        let keypair2 = KasEcKeypair::from_private_key_bytes(&private_bytes).unwrap();

        // Public keys should match
        assert_eq!(keypair1.public_key_pem(), keypair2.public_key_pem());
    }

    #[test]
    fn test_parse_public_key_pem() {
        let keypair = KasEcKeypair::generate().unwrap();
        let pem = keypair.public_key_pem();

        let parsed = KasEcKeypair::parse_public_key_pem(pem).unwrap();
        assert_eq!(keypair.public_key(), &parsed);
    }

    #[cfg(feature = "rsa")]
    #[test]
    fn test_rsa_keypair_generate() {
        let keypair = KasRsaKeypair::generate().unwrap();
        assert!(!keypair.public_key_pem().is_empty());
        assert!(keypair.public_key_pem().contains("PUBLIC KEY"));
    }
}
