//! Unwrap-on-load (spec §13.2): open, bind, and verify before serving bytes.
//!
//! `open` needs only the zip and the manifest, so a malformed archive is
//! rejected before any KAS round-trip. `unlock` performs the single rewrap,
//! decrypts the header, binds the plaintext index to it (§9.5), and verifies
//! the root signature (§10.4) before any weight byte can be served.

use crate::error::GgufTdfError;
use crate::gguf_header::parse_header;
use crate::index::{SegmentMap, validate_index};
use crate::key::PayloadKeyUnwrapper;
use crate::read_at::VirtualGguf;
use crate::{MANIFEST_ENTRY, MANIFEST_ENTRY_FALLBACK, PROFILE};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use opentdf::{GgufIndex, TdfEncryption, TdfManifest, TdfMemberIndex};
use sha2::Sha256;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// An opened `.gguf.tdf` whose structure has been validated but whose payload
/// key has not been requested yet.
pub struct GgufTdfArchive {
    file: File,
    members: TdfMemberIndex,
    manifest: TdfManifest,
    map: SegmentMap,
}

impl GgufTdfArchive {
    /// Opens and structurally validates an archive (spec §13.2 steps 1–2).
    ///
    /// No payload key is needed and none is requested.
    pub fn open(path: &Path) -> Result<Self, GgufTdfError> {
        let mut file = File::open(path)?;
        let members = TdfMemberIndex::open(&mut file).map_err(map_member_open_error)?;
        if members.contains("0.payload") {
            return Err(GgufTdfError::PayloadForbidden);
        }

        let manifest = read_manifest(&mut file, &members)?;
        let index = manifest
            .gguf
            .as_ref()
            .ok_or_else(|| GgufTdfError::UnsupportedProfile("<absent>".to_string()))?;
        if index.profile != PROFILE {
            return Err(GgufTdfError::UnsupportedProfile(index.profile.clone()));
        }

        let map = validate_index(
            index,
            &manifest.encryption_information.integrity_information,
            &members,
        )?;

        Ok(Self {
            file,
            members,
            manifest,
            map,
        })
    }

    /// Length of the virtual GGUF this archive serves.
    pub fn virtual_size(&self) -> u64 {
        self.index().virtual_size
    }

    /// Offset of `tensor_data` in the virtual file.
    pub fn header_bytes(&self) -> u64 {
        self.index().header_bytes
    }

    /// Maximum plaintext size of a non-header segment.
    pub fn max_segment(&self) -> u64 {
        self.index().max_segment
    }

    /// The parsed manifest, for a KAS client that needs `keyAccess`.
    pub fn manifest(&self) -> &TdfManifest {
        &self.manifest
    }

    fn index(&self) -> &GgufIndex {
        self.manifest
            .gguf
            .as_ref()
            .expect("open() rejects an archive without a gguf index")
    }

    /// Rewraps the payload key and prepares the virtual GGUF (§13.2 steps 3–6).
    ///
    /// Verifies the header's GMAC, binds the plaintext index to the
    /// authenticated header, and checks the root signature, all before any
    /// caller can read a weight byte.
    pub fn unlock(
        mut self,
        unwrapper: &dyn PayloadKeyUnwrapper,
    ) -> Result<VirtualGguf, GgufTdfError> {
        let payload_key = Zeroizing::new(unwrapper.unwrap_key(&self.manifest)?);

        let encryption = TdfEncryption::with_payload_key(payload_key.as_ref())
            .map_err(|e| GgufTdfError::KasDenied(format!("KAS returned an unusable key: {e}")))?;

        let header_plain =
            decrypt_header(&mut self.file, &self.members, &self.manifest, &encryption)?;
        bind_index_to_header(self.index(), &header_plain)?;
        verify_root_signature(&self.manifest, &payload_key)?;

        let index = self.index().clone();
        let hashes = self
            .manifest
            .encryption_information
            .integrity_information
            .segments
            .iter()
            .map(|s| s.hash.clone())
            .collect();

        Ok(VirtualGguf::new(
            self.file,
            self.members,
            index,
            self.map,
            encryption,
            header_plain,
            hashes,
        ))
    }
}

fn map_member_open_error(err: opentdf::TdfError) -> GgufTdfError {
    match err {
        opentdf::TdfError::ZipError(zip::result::ZipError::InvalidArchive(_)) => {
            GgufTdfError::NotZip
        }
        other => other.into(),
    }
}

/// Reads `0.manifest.json`, falling back to `manifest.json` (spec §6.5).
fn read_manifest(file: &mut File, members: &TdfMemberIndex) -> Result<TdfManifest, GgufTdfError> {
    let location = match members.get(MANIFEST_ENTRY) {
        Some(loc) => {
            if members.contains(MANIFEST_ENTRY_FALLBACK) {
                tracing::debug!("archive has both manifest names; ignoring manifest.json");
            }
            loc
        }
        None => members
            .get(MANIFEST_ENTRY_FALLBACK)
            .ok_or(GgufTdfError::NoManifest)?,
    };

    let mut json = vec![0u8; location.size as usize];
    file.seek(SeekFrom::Start(location.data_start))?;
    file.read_exact(&mut json)?;

    let text = std::str::from_utf8(&json)
        .map_err(|e| GgufTdfError::BadIndex(format!("manifest is not UTF-8: {e}")))?;
    TdfManifest::from_json(text)
        .map_err(|e| GgufTdfError::BadIndex(format!("manifest is not valid JSON: {e}")))
}

/// Decrypts zip member `header` into a retained plaintext buffer.
fn decrypt_header(
    file: &mut File,
    members: &TdfMemberIndex,
    manifest: &TdfManifest,
    encryption: &TdfEncryption,
) -> Result<Zeroizing<Vec<u8>>, GgufTdfError> {
    let location = members
        .get(crate::HEADER_ENTRY)
        .ok_or_else(|| GgufTdfError::BadIndex("archive has no header member".to_string()))?;

    let mut member = vec![0u8; location.size as usize];
    file.seek(SeekFrom::Start(location.data_start))?;
    file.read_exact(&mut member)?;

    let row = manifest
        .encryption_information
        .integrity_information
        .segments
        .first()
        .ok_or_else(|| GgufTdfError::BadIndex("no integrity row for the header".to_string()))?;
    let plain_len = row
        .segment_size
        .ok_or_else(|| GgufTdfError::BadIndex("header row omits segmentSize".to_string()))?;

    let mut plaintext = Zeroizing::new(vec![0u8; plain_len as usize]);
    let tag = encryption
        .decrypt_segment_into(&member, &mut plaintext)
        .map_err(|_| GgufTdfError::TagMismatch)?;

    // The GCM decrypt already authenticated the tag; comparing it with the
    // manifest catches a tag or manifest swap.
    let expected = base64::engine::general_purpose::STANDARD
        .decode(&row.hash)
        .map_err(|_| GgufTdfError::TagMismatch)?;
    if expected.ct_eq(&tag).unwrap_u8() != 1 {
        return Err(GgufTdfError::TagMismatch);
    }

    Ok(plaintext)
}

/// Spec §9.5: bind the plaintext index to the authenticated header.
fn bind_index_to_header(index: &GgufIndex, header_plain: &[u8]) -> Result<(), GgufTdfError> {
    let parsed = parse_header(&mut Cursor::new(header_plain))?;

    if parsed.data_offset != index.header_bytes {
        return Err(GgufTdfError::BadIndex(format!(
            "header says tensor_data is at {}, index says {}",
            parsed.data_offset, index.header_bytes
        )));
    }
    if parsed.alignment != index.alignment {
        return Err(GgufTdfError::BadIndex(format!(
            "header alignment {} disagrees with index alignment {}",
            parsed.alignment, index.alignment
        )));
    }
    if parsed.tensors.len() != index.tensors.len() {
        return Err(GgufTdfError::BadIndex(format!(
            "header has {} tensors, index has {}",
            parsed.tensors.len(),
            index.tensors.len()
        )));
    }

    for (from_header, from_index) in parsed.tensors.iter().zip(&index.tensors) {
        if from_header.name != from_index.name {
            return Err(GgufTdfError::BadIndex(format!(
                "tensor name {:?} in the header, {:?} in the index",
                from_header.name, from_index.name
            )));
        }
        if index.header_bytes + from_header.gguf_offset != from_index.offset {
            return Err(GgufTdfError::BadIndex(format!(
                "tensor {:?} offset disagrees with the header",
                from_header.name
            )));
        }
        if from_header.size != from_index.size {
            return Err(GgufTdfError::BadIndex(format!(
                "tensor {:?} size disagrees with the header",
                from_header.name
            )));
        }
    }

    Ok(())
}

/// Spec §10.4: HMAC over the concatenated raw 16-byte segment tags.
///
/// GMAC authenticates each member in isolation and does not bind order, so an
/// equal-size swap of two members with their `hash` rows is caught only here.
/// The manifest's decoded hashes are used, so no unused member is decrypted.
fn verify_root_signature(
    manifest: &TdfManifest,
    payload_key: &[u8; 32],
) -> Result<(), GgufTdfError> {
    let integrity = &manifest.encryption_information.integrity_information;

    let mut mac = <Hmac<Sha256>>::new_from_slice(payload_key)
        .map_err(|_| GgufTdfError::BadIndex("invalid payload key length".to_string()))?;
    for row in &integrity.segments {
        let tag = base64::engine::general_purpose::STANDARD
            .decode(&row.hash)
            .map_err(|_| GgufTdfError::RootMismatch)?;
        if tag.len() != 16 {
            return Err(GgufTdfError::RootMismatch);
        }
        mac.update(&tag);
    }
    let computed = mac.finalize().into_bytes();

    let expected = base64::engine::general_purpose::STANDARD
        .decode(&integrity.root_signature.sig)
        .map_err(|_| GgufTdfError::RootMismatch)?;
    if expected.ct_eq(computed.as_slice()).unwrap_u8() != 1 {
        return Err(GgufTdfError::RootMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_zip_magic_is_not_zip() {
        let err = map_member_open_error(opentdf::TdfError::ZipError(
            zip::result::ZipError::InvalidArchive("Invalid zip header"),
        ));
        assert_eq!(err.code(), "GGUFTDF_NOT_ZIP");
    }

    #[test]
    fn a_stored_layout_violation_is_not_reported_as_not_zip() {
        let err = map_member_open_error(opentdf::TdfError::InvalidStructure {
            reason: "member 'header' is not Stored".to_string(),
            expected: Some("compression method 0".to_string()),
        });
        assert_eq!(err.code(), "GGUFTDF_BAD_INDEX");
    }
}
