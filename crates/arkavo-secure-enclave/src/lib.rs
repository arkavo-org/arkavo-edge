//! P-256 signing keys held in the Apple Secure Enclave.
//!
//! The private key never leaves the enclave. CryptoKit hands back a wrapped
//! form of it that only this Mac's enclave can use, so the caller keeps that
//! in a file and signs with it again after a restart. Keys are made for
//! unattended use: signing needs no user presence, biometry or passcode, only
//! that someone has logged in since the Mac started.
//!
//! The wrapped key is bound to the Mac, not to a process: anything on this
//! Mac that can read the file can sign with it, so the caller keeps the file
//! readable by its owner only.
#![cfg(target_os = "macos")]

use std::fmt;

unsafe extern "C" {
    fn arkavo_se_available() -> bool;
    fn arkavo_se_create(blob: *mut u8, capacity: usize, length: *mut usize) -> i32;
    fn arkavo_se_public_key(blob: *const u8, length: usize, point: *mut u8) -> i32;
    fn arkavo_se_sign(
        blob: *const u8,
        length: usize,
        message: *const u8,
        message_length: usize,
        signature: *mut u8,
    ) -> i32;
}

/// Why the enclave did not do what was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// This Mac has no Secure Enclave, or this process cannot reach it.
    Unavailable,
    /// The enclave refused: the wrapped key belongs to another Mac, or nobody
    /// has logged in since the Mac started.
    Refused,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "this Mac has no Secure Enclave this process can use",
            Self::Refused => {
                "the Secure Enclave refused the key: it belongs to another Mac, \
                 or nobody has logged in since this Mac started"
            }
        })
    }
}

impl std::error::Error for Error {}

fn status(code: i32) -> Result<(), Error> {
    match code {
        0 => Ok(()),
        1 => Err(Error::Unavailable),
        _ => Err(Error::Refused),
    }
}

/// Whether this Mac has a Secure Enclave this process can use.
pub fn is_available() -> bool {
    // SAFETY: takes no arguments and only reads CryptoKit state.
    unsafe { arkavo_se_available() }
}

/// A P-256 private key wrapped by this Mac's Secure Enclave.
#[derive(Clone, PartialEq, Eq)]
pub struct WrappedKey(Vec<u8>);

impl WrappedKey {
    /// Create a key inside the enclave and return its wrapped form.
    pub fn create() -> Result<Self, Error> {
        // CryptoKit's wrapped keys are a few hundred bytes.
        let mut blob = vec![0u8; 1024];
        let mut length = 0usize;
        // SAFETY: the shim writes at most `blob.len()` bytes into `blob` and
        // the count into `length`, and keeps neither pointer.
        status(unsafe { arkavo_se_create(blob.as_mut_ptr(), blob.len(), &raw mut length) })?;
        blob.truncate(length);
        Ok(Self(blob))
    }

    /// A wrapped key read back from where the caller kept it.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The public key as a 65-byte uncompressed SEC1 point.
    pub fn public_key(&self) -> Result<[u8; 65], Error> {
        let mut point = [0u8; 65];
        // SAFETY: the shim reads `self.0.len()` bytes and writes exactly 65.
        status(unsafe { arkavo_se_public_key(self.0.as_ptr(), self.0.len(), point.as_mut_ptr()) })?;
        Ok(point)
    }

    /// ECDSA P-256 with SHA-256 over `message`, as the 64-byte `r || s`
    /// that JWS ES256 and COSE carry.
    pub fn sign(&self, message: &[u8]) -> Result<[u8; 64], Error> {
        let mut signature = [0u8; 64];
        // SAFETY: the shim reads the given lengths and writes exactly 64 bytes.
        status(unsafe {
            arkavo_se_sign(
                self.0.as_ptr(),
                self.0.len(),
                message.as_ptr(),
                message.len(),
                signature.as_mut_ptr(),
            )
        })?;
        Ok(signature)
    }
}

impl fmt::Debug for WrappedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WrappedKey({} bytes)", self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Verifier;
    use p256::ecdsa::{Signature, VerifyingKey};

    #[test]
    fn a_key_from_elsewhere_is_refused_not_trusted() {
        let foreign = WrappedKey::from_bytes(vec![7u8; 324]);
        assert_eq!(foreign.public_key(), Err(Error::Refused));
        assert_eq!(foreign.sign(b"payload"), Err(Error::Refused));
    }

    #[test]
    #[ignore = "requires a Secure Enclave"]
    fn a_kept_key_signs_unattended_after_reloading() {
        assert!(is_available());
        let created = WrappedKey::create().expect("create");
        let reloaded = WrappedKey::from_bytes(created.as_bytes().to_vec());
        let point = reloaded.public_key().expect("public key");
        assert_eq!(point, created.public_key().expect("public key"));

        let signature = reloaded.sign(b"agent card").expect("sign");
        let key = VerifyingKey::from_sec1_bytes(&point).expect("point");
        let signature = Signature::from_slice(&signature).expect("r || s");
        assert!(key.verify(b"agent card", &signature).is_ok());
        assert!(key.verify(b"another card", &signature).is_err());
    }
}
