use sha2::Digest;

/// Pluggable digest algorithm for permit-bound hashes (policy bundle,
/// canonical arguments, sequence state). SHA-256 is the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HashAlgorithm {
    #[default]
    Sha256,
    Blake3,
}

impl HashAlgorithm {
    pub fn digest(self, data: &[u8]) -> Vec<u8> {
        match self {
            Self::Sha256 => sha2::Sha256::digest(data).to_vec(),
            Self::Blake3 => blake3::hash(data).as_bytes().to_vec(),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
            Self::Blake3 => "blake3",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "sha256" => Some(Self::Sha256),
            "blake3" => Some(Self::Blake3),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vector() {
        // SHA-256("abc") per FIPS 180-4.
        let digest = HashAlgorithm::Sha256.digest(b"abc");
        assert_eq!(
            digest,
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad
            ]
        );
    }

    #[test]
    fn blake3_matches_known_vector() {
        // BLAKE3("abc") from the official BLAKE3 test vectors.
        let digest = HashAlgorithm::Blake3.digest(b"abc");
        assert_eq!(
            digest,
            [
                0x64, 0x37, 0xb3, 0xac, 0x38, 0x46, 0x51, 0x33, 0xff, 0xb6, 0x3b, 0x75, 0x27, 0x3a,
                0x8d, 0xb5, 0x48, 0xc5, 0x58, 0x46, 0x5d, 0x79, 0xdb, 0x03, 0xfd, 0x35, 0x9c, 0x6c,
                0xd5, 0xbd, 0x9d, 0x85
            ]
        );
    }

    #[test]
    fn name_roundtrip() {
        assert_eq!(
            HashAlgorithm::from_name("sha256"),
            Some(HashAlgorithm::Sha256)
        );
        assert_eq!(
            HashAlgorithm::from_name("blake3"),
            Some(HashAlgorithm::Blake3)
        );
        assert_eq!(HashAlgorithm::from_name("md5"), None);
        for alg in [HashAlgorithm::Sha256, HashAlgorithm::Blake3] {
            assert_eq!(HashAlgorithm::from_name(alg.name()), Some(alg));
        }
    }
}
