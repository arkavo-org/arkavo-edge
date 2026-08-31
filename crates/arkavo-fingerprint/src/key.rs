//! The tenant index key (KP-009).
//!
//! Every hash written to or looked up in a reference index is keyed with this.
//! An unkeyed index of sensitive content is a dictionary: anyone holding it can
//! confirm a guess by hashing the guess and looking for the digest. Keying is
//! what makes the index useless to whoever steals it, so there is deliberately
//! no way to compute an entry without one.

use zeroize::{Zeroize, ZeroizeOnDrop};

/// Domain separator for key derivation. Changing it invalidates every index.
const DERIVE_CONTEXT: &str = "arkavo-fingerprint 2026-08 tenant index key";

/// Digest width kept per shingle.
///
/// Eight bytes, not thirty-two. The index holds one of these per shingle of the
/// corpus, so width is the dominant cost; against a keyed PRF the attacker
/// cannot grind offline, which is what would otherwise force a wider digest.
pub type ShingleHash = u64;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("tenant index key material must be at least {MIN_SECRET_BYTES} bytes, got {0}")]
    SecretTooShort(usize),
}

/// Shortest tenant secret accepted. Below this the key is guessable and the
/// index reverts to the dictionary the keying exists to prevent.
pub const MIN_SECRET_BYTES: usize = 16;

/// A tenant's index key.
///
/// Zeroized on drop. It is derived from material the KAS released, and a copy
/// left in freed memory outlives the entitlement that released it.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct IndexKey {
    material: [u8; 32],
}

impl std::fmt::Debug for IndexKey {
    /// Never prints the key. A struct that renders its own secret ends up in a
    /// log the first time someone debugs the thing that holds it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IndexKey(<redacted>)")
    }
}

impl IndexKey {
    /// Derive a key from tenant secret material.
    ///
    /// `index_id` separates indices under one tenant secret, so a match in one
    /// index cannot be replayed against another.
    pub fn derive(secret: &[u8], index_id: &str) -> Result<Self, KeyError> {
        if secret.len() < MIN_SECRET_BYTES {
            return Err(KeyError::SecretTooShort(secret.len()));
        }
        let context = format!("{DERIVE_CONTEXT} :: {index_id}");
        let mut material = blake3::derive_key(&context, secret);
        let key = Self { material };
        material.zeroize();
        Ok(key)
    }

    /// Wrap key material that was derived elsewhere.
    pub fn from_material(material: [u8; 32]) -> Self {
        Self { material }
    }

    /// Keyed digest of one shingle.
    ///
    /// BLAKE3's keyed mode is a PRF over the input in a single pass, so this is
    /// the per-shingle cost on the hot path — the thing the budget is spent on.
    pub fn hash(&self, shingle: &str) -> ShingleHash {
        let digest = blake3::keyed_hash(&self.material, shingle.as_bytes());
        let bytes = digest.as_bytes();
        u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])
    }

    /// Fingerprint of the key itself, for binding an index to the key that
    /// built it. Derived through the PRF, so it reveals nothing about the key.
    pub fn fingerprint(&self) -> String {
        let digest = blake3::keyed_hash(&self.material, b"arkavo-fingerprint key id");
        digest.to_hex().as_str()[..16].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(seed: u8) -> Vec<u8> {
        (0..32).map(|i| i as u8 ^ seed).collect()
    }

    #[test]
    fn the_same_secret_and_id_derive_the_same_key() {
        let a = IndexKey::derive(&secret(1), "corpus-a").expect("derive");
        let b = IndexKey::derive(&secret(1), "corpus-a").expect("derive");

        assert_eq!(a.hash("hello world"), b.hash("hello world"));
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn a_different_index_id_derives_a_different_key() {
        // Otherwise a hit in one index confirms content in another.
        let a = IndexKey::derive(&secret(1), "corpus-a").expect("derive");
        let b = IndexKey::derive(&secret(1), "corpus-b").expect("derive");

        assert_ne!(a.hash("hello world"), b.hash("hello world"));
    }

    #[test]
    fn a_different_tenant_secret_derives_a_different_key() {
        let a = IndexKey::derive(&secret(1), "corpus").expect("derive");
        let b = IndexKey::derive(&secret(2), "corpus").expect("derive");

        assert_ne!(a.hash("hello world"), b.hash("hello world"));
    }

    #[test]
    fn short_key_material_is_refused() {
        // A guessable key is the same as no key.
        assert_eq!(
            IndexKey::derive(b"short", "corpus").unwrap_err(),
            KeyError::SecretTooShort(5)
        );
    }

    #[test]
    fn distinct_shingles_hash_distinctly() {
        let key = IndexKey::derive(&secret(1), "corpus").expect("derive");

        assert_ne!(key.hash("the quick brown"), key.hash("quick brown fox"));
    }

    #[test]
    fn the_key_never_renders_itself() {
        let key = IndexKey::derive(&secret(1), "corpus").expect("derive");

        let rendered = format!("{key:?}");

        assert_eq!(rendered, "IndexKey(<redacted>)");
        assert!(!rendered.contains(&key.fingerprint()));
    }
}
