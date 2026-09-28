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
use crate::prefetch::Prefetcher;
use crate::read_at::VirtualGguf;
use crate::{
    MANIFEST_ENTRY, MANIFEST_ENTRY_FALLBACK, MAX_HEADER_BYTES, MAX_MANIFEST_BYTES, PROFILE,
    SEGMENT_OVERHEAD,
};
use base64::Engine as _;
use opentdf::manifest::IntegrityInformationExt;
use opentdf::{GgufIndex, TdfEncryption, TdfManifest, TdfMemberIndex};
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// An opened `.gguf.tdf` whose structure has been validated but whose payload
/// key has not been requested yet.
pub struct GgufTdfArchive {
    /// Kept so the decrypt-ahead worker can open its own handle; `try_clone`
    /// would only `dup` this one, and a duplicated descriptor shares the seek
    /// cursor, so the worker and the reader would race on every read.
    path: PathBuf,
    file: File,
    members: TdfMemberIndex,
    manifest: TdfManifest,
    map: SegmentMap,
    /// What this archive is within a pack, when it says. Read at `open`, so it
    /// is available before any key is requested.
    #[cfg(feature = "knowledge-pack")]
    component: Option<crate::component::ComponentMetadata>,
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

        #[cfg(feature = "knowledge-pack")]
        let component = read_component(&mut file, &members)?;

        Ok(Self {
            path: path.to_path_buf(),
            file,
            members,
            manifest,
            #[cfg(feature = "knowledge-pack")]
            component,
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

    /// What this archive is within a pack, when it says.
    ///
    /// Available before `unlock`, which is the point: an egress node decides
    /// whether it is entitled to ask for this component's key by reading this,
    /// and that decision necessarily precedes any decryption.
    #[cfg(feature = "knowledge-pack")]
    pub fn component(&self) -> Option<&crate::component::ComponentMetadata> {
        self.component.as_ref()
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
    pub fn unlock(self, unwrapper: &dyn PayloadKeyUnwrapper) -> Result<VirtualGguf, GgufTdfError> {
        self.unlock_with_options(
            unwrapper,
            crate::DEFAULT_CACHED_SEGMENTS,
            crate::DEFAULT_PREFETCH_DEPTH,
        )
    }

    /// `unlock` with an explicit number of decrypted weight segments to keep
    /// (spec §13.3) and no decrypt-ahead worker, so extra plaintext is exactly
    /// `headerBytes + cached_segments * maxSegment`. `cached_segments == 0` is
    /// treated as 1.
    pub fn unlock_with_cache(
        self,
        unwrapper: &dyn PayloadKeyUnwrapper,
        cached_segments: usize,
    ) -> Result<VirtualGguf, GgufTdfError> {
        self.unlock_with_options(unwrapper, cached_segments, 0)
    }

    /// `unlock` with both reader knobs (spec §13.3).
    ///
    /// `prefetch_depth` segments are decrypted ahead of the loader on a
    /// worker thread, so AES overlaps the loader's copy; `0` spawns no
    /// thread. Extra plaintext is `headerBytes + (cached_segments +
    /// prefetch_depth) * maxSegment`.
    pub fn unlock_with_options(
        mut self,
        unwrapper: &dyn PayloadKeyUnwrapper,
        cached_segments: usize,
        prefetch_depth: usize,
    ) -> Result<VirtualGguf, GgufTdfError> {
        let payload_key = Zeroizing::new(unwrapper.unwrap_key(&self.manifest)?);

        let encryption = TdfEncryption::with_payload_key(payload_key.as_ref())
            .map_err(|e| GgufTdfError::KasDenied(format!("KAS returned an unusable key: {e}")))?;

        let header_plain =
            decrypt_header(&mut self.file, &self.members, &self.manifest, &encryption)?;
        bind_index_to_header(self.index(), &header_plain)?;
        verify_root_signature(&self.manifest, &payload_key)?;

        let index = self.index().clone();
        let hashes: Vec<String> = self
            .manifest
            .encryption_information
            .integrity_information
            .segments
            .iter()
            .map(|s| s.hash.clone())
            .collect();

        // The worker reads and decrypts independently of the reader, so it
        // gets its own open file and its own cipher state; everything it
        // needs is already authenticated above. Should this open see a
        // different file than `open` did, every byte it produces is still
        // checked against the manifest's GMAC row, so the swap can only fail
        // closed for the segment concerned.
        let prefetch = if prefetch_depth > 0 {
            Some(Prefetcher::spawn(
                &self.path,
                TdfEncryption::with_payload_key(payload_key.as_ref()).map_err(|e| {
                    GgufTdfError::KasDenied(format!("KAS returned an unusable key: {e}"))
                })?,
                self.members.clone(),
                index.segments.clone(),
                hashes.clone(),
                prefetch_depth,
            )?)
        } else {
            None
        };

        Ok(VirtualGguf::new(
            self.file,
            self.members,
            index,
            self.map,
            encryption,
            header_plain,
            hashes,
            cached_segments,
            prefetch,
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

/// Largest component-metadata member accepted.
///
/// Small on purpose: this is a handful of short strings, and it is parsed
/// before anything has been authenticated, so the bound is what stops a
/// hostile archive from making a reader allocate on its say-so.
#[cfg(feature = "knowledge-pack")]
const MAX_COMPONENT_BYTES: u64 = 64 * 1024;

/// Read the plaintext component member, if the archive carries one.
///
/// An archive without one is not an error: every artifact wrapped before this
/// member existed has none, and the runtime treats a missing ceiling
/// conservatively rather than refusing to open the file.
#[cfg(feature = "knowledge-pack")]
fn read_component(
    file: &mut File,
    members: &TdfMemberIndex,
) -> Result<Option<crate::component::ComponentMetadata>, GgufTdfError> {
    let Some(location) = members.get(crate::COMPONENT_ENTRY) else {
        return Ok(None);
    };
    if location.size > MAX_COMPONENT_BYTES {
        return Err(GgufTdfError::BadIndex(format!(
            "component member is {} bytes, over the {MAX_COMPONENT_BYTES} byte cap",
            location.size
        )));
    }
    let mut json = vec![0u8; location.size as usize];
    file.seek(SeekFrom::Start(location.data_start))?;
    file.read_exact(&mut json)?;
    serde_json::from_slice(&json)
        .map(Some)
        .map_err(|e| GgufTdfError::BadIndex(format!("component metadata: {e}")))
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

    if location.size > MAX_MANIFEST_BYTES {
        return Err(GgufTdfError::BadIndex(format!(
            "manifest member is {} bytes, over the {MAX_MANIFEST_BYTES} byte cap",
            location.size
        )));
    }

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

    let row = manifest
        .encryption_information
        .integrity_information
        .segments
        .first()
        .ok_or_else(|| GgufTdfError::BadIndex("no integrity row for the header".to_string()))?;
    let plain_len = row
        .segment_size
        .ok_or_else(|| GgufTdfError::BadIndex("header row omits segmentSize".to_string()))?;

    // Checked before either buffer below is allocated. `validate_index`
    // already bounds `headerBytes`, but this function does not assume its
    // caller ran that check, so it enforces the same cap on the two sizes
    // it is about to allocate from.
    check_header_size_caps(plain_len, location.size)?;

    let mut member = vec![0u8; location.size as usize];
    file.seek(SeekFrom::Start(location.data_start))?;
    file.read_exact(&mut member)?;

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

/// Task 10 cap, split out so both branches are unit-testable without a real
/// multi-hundred-MiB or multi-GiB fixture: `plain_len` is the row's claimed
/// `segmentSize`, `member_len` the header member's on-disk size.
fn check_header_size_caps(plain_len: u64, member_len: u64) -> Result<(), GgufTdfError> {
    if plain_len > MAX_HEADER_BYTES {
        return Err(GgufTdfError::BadIndex(format!(
            "header row claims segmentSize {plain_len}, over the {MAX_HEADER_BYTES} byte cap"
        )));
    }
    let member_cap = MAX_HEADER_BYTES + SEGMENT_OVERHEAD;
    if member_len > member_cap {
        return Err(GgufTdfError::BadIndex(format!(
            "header member is {member_len} bytes, over the {member_cap} byte cap"
        )));
    }
    Ok(())
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

/// Spec §10.4: HMAC over the concatenated raw 16-byte segment tags, verified
/// through `opentdf`'s own constant-time implementation rather than a local
/// HMAC (opentdf-rs#100 item 2 would also retire the row decoding below).
///
/// GMAC authenticates each member in isolation and does not bind order, so an
/// equal-size swap of two members with their `hash` rows is caught only here.
/// The manifest's decoded hashes are used, so no unused member is decrypted.
fn verify_root_signature(
    manifest: &TdfManifest,
    payload_key: &[u8; 32],
) -> Result<(), GgufTdfError> {
    let integrity = &manifest.encryption_information.integrity_information;
    let mut tags = Vec::with_capacity(integrity.segments.len());
    for row in &integrity.segments {
        let tag = base64::engine::general_purpose::STANDARD
            .decode(&row.hash)
            .map_err(|_| GgufTdfError::RootMismatch)?;
        // The library concatenates whatever it is given; the profile requires
        // exactly one 16-byte GMAC per row (§10.4), so enforce that here.
        if tag.len() != 16 {
            return Err(GgufTdfError::RootMismatch);
        }
        tags.push(tag);
    }
    integrity
        .verify_root_signature(&tags, payload_key)
        .map_err(|_| GgufTdfError::RootMismatch)
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

    // Task 10: absurd header/segment sizes are refused before allocating.
    // `check_header_size_caps` is exercised directly for both branches
    // because the second (a member whose on-disk size itself exceeds the
    // cap) needs a real >1 GiB zip member to reach through `decrypt_header`
    // — infeasible as a test fixture. The first branch also gets a
    // `decrypt_header`-level test below, with a real (tiny) member and a
    // manifest row that only lies about `segmentSize`.

    #[test]
    fn header_size_caps_accept_values_at_the_boundary() {
        check_header_size_caps(MAX_HEADER_BYTES, MAX_HEADER_BYTES + SEGMENT_OVERHEAD)
            .expect("exactly at the cap must be accepted");
    }

    #[test]
    fn header_size_caps_reject_a_segment_size_claim_over_the_cap() {
        let err = check_header_size_caps(MAX_HEADER_BYTES + 1, 64).unwrap_err();
        assert_eq!(err.code(), "GGUFTDF_BAD_INDEX");
    }

    #[test]
    fn header_size_caps_reject_a_member_size_over_the_cap() {
        let err = check_header_size_caps(64, MAX_HEADER_BYTES + SEGMENT_OVERHEAD + 1).unwrap_err();
        assert_eq!(err.code(), "GGUFTDF_BAD_INDEX");
    }

    #[test]
    fn decrypt_header_refuses_a_segment_size_claim_over_the_cap_before_allocating() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bare-header.zip");
        let encryption = TdfEncryption::with_payload_key(&[0x5A; 32]).unwrap();
        let encrypted = encryption.encrypt_segment(b"a tiny real header").unwrap();

        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        zip.start_file(crate::HEADER_ENTRY, options).unwrap();
        zip.write_all(&encrypted.bytes).unwrap();
        zip.finish().unwrap();

        let mut file = File::open(&path).unwrap();
        let members = opentdf::TdfMemberIndex::open(&mut file).unwrap();

        let mut manifest = TdfManifest::new(
            crate::HEADER_ENTRY.to_string(),
            "https://kas.invalid".to_string(),
        );
        manifest
            .encryption_information
            .integrity_information
            .segments = vec![opentdf::Segment {
            hash: base64::engine::general_purpose::STANDARD.encode(encrypted.tag),
            // The member on disk is a few bytes; only the claim is absurd.
            segment_size: Some(MAX_HEADER_BYTES + 32),
            encrypted_segment_size: Some(MAX_HEADER_BYTES + 32 + SEGMENT_OVERHEAD),
        }];

        let err = decrypt_header(&mut file, &members, &manifest, &encryption).unwrap_err();
        assert_eq!(err.code(), "GGUFTDF_BAD_INDEX");
    }
}
