//! Wrap procedure (spec §12).
//!
//! Each planned segment is read from the source with a seek plus one bounded
//! read, so a multi-gigabyte model is never buffered. The manifest is written
//! last because its root signature covers every segment tag.

use crate::error::GgufTdfError;
use crate::gguf_header::parse_header;
use crate::index::build_index;
use crate::key::{PayloadKeyWrapper, PreResolvedKey};
use crate::pack::plan_segments;
use crate::reader::GgufTdfArchive;
use crate::{DEFAULT_MAX_SEGMENT, MANIFEST_ENTRY, SEGMENT_OVERHEAD};
use opentdf::manifest::{IntegrityInformationExt, KeyAccessExt};
use opentdf::{Segment, TdfEncryption, TdfManifest, TdfMultiEntryBuilder};
use rand::RngCore;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// OpenTDF specification version whose crypto and manifest semantics this
/// profile matches.
const TDF_SPEC_VERSION: &str = "4.3.0";

/// Options for [`protect`].
#[derive(Debug, Clone)]
pub struct ProtectOptions {
    /// Maximum plaintext size of a non-header segment.
    pub max_segment: u64,
    /// Attribute FQNs to place in the policy's `dataAttributes`.
    pub attributes: Vec<String>,
    /// Dissemination list; empty is normal for a model artifact.
    pub dissem: Vec<String>,
    /// `payload.mimeType`: the original unencrypted type.
    pub mime_type: String,
}

impl Default for ProtectOptions {
    fn default() -> Self {
        Self {
            max_segment: DEFAULT_MAX_SEGMENT,
            attributes: Vec::new(),
            dissem: Vec::new(),
            mime_type: "application/x-gguf".to_string(),
        }
    }
}

/// What a successful wrap produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectReport {
    /// Number of encrypted members, including the header.
    pub segments: usize,
    /// Length of the source GGUF, which is also the virtual size.
    pub virtual_size: u64,
    /// Offset of `tensor_data` in the virtual file.
    pub header_bytes: u64,
    /// Size of the written archive.
    pub archive_bytes: u64,
    /// `true` once the archive has been reopened and unlocked with the fresh
    /// payload key, so `--delete-source` has proof the archive it is about
    /// to leave as the only copy actually unlocks. `protect` never returns
    /// `Ok` with this `false`; the field exists so a caller cannot delete a
    /// source on a report it did not check.
    pub verified_header: bool,
}

/// Wraps `source` (a little-endian GGUF v3 file) into `dest` as a
/// `gguf-tdf/1` archive.
///
/// The source is never deleted or modified; wrapping is additive.
pub fn protect(
    source: &Path,
    dest: &Path,
    wrapper: &dyn PayloadKeyWrapper,
    opts: &ProtectOptions,
) -> Result<ProtectReport, GgufTdfError> {
    let mut file = BufReader::new(File::open(source)?);
    let virtual_size = std::fs::metadata(source)?.len();

    let header = parse_header(&mut file)?;
    let plan = plan_segments(&header, virtual_size, opts.max_segment)?;
    let index = build_index(&header, &plan, virtual_size, opts.max_segment)?;

    let mut payload_key = Zeroizing::new([0u8; 32]);
    rand::rngs::OsRng.fill_bytes(payload_key.as_mut());

    let wrapped = wrapper.wrap(&payload_key)?;
    let encryption = TdfEncryption::with_payload_key(payload_key.as_ref())
        .map_err(|e| GgufTdfError::Crypto(format!("payload key rejected: {e}")))?;

    let staging = staging_path(dest);
    if staging.exists() {
        std::fs::remove_file(&staging)?;
    }

    let written = write_archive(
        &mut file,
        &staging,
        &plan,
        &index,
        &payload_key,
        &wrapped,
        &encryption,
        opts,
        virtual_size,
        header.data_offset,
    );
    match written {
        Ok(mut report) => {
            if let Err(err) = std::fs::rename(&staging, dest) {
                let _ = std::fs::remove_file(&staging);
                return Err(err.into());
            }
            // dest is now the only artifact; a read-back failure here must
            // leave it in place for inspection rather than delete it, and
            // must never touch the source. The caller (--delete-source)
            // trusts `verified_header` alone, so it is only ever set on the
            // success path below.
            read_back(dest, &payload_key)?;
            report.verified_header = true;
            Ok(report)
        }
        Err(err) => {
            let _ = std::fs::remove_file(&staging);
            Err(err)
        }
    }
}

/// Proves the archive just written unlocks with the key it was wrapped
/// with: structure, header GMAC, and root signature. A single cached
/// segment is enough since only the header member is read; no worker thread
/// is spawned. A wrap that cannot be read back must never be reported as
/// success — `--delete-source` relies on this before it removes the only
/// plaintext copy.
fn read_back(dest: &Path, payload_key: &[u8; 32]) -> Result<(), GgufTdfError> {
    GgufTdfArchive::open(dest)?
        .unlock_with_cache(&PreResolvedKey::new(*payload_key), 1)
        .map_err(|e| {
            GgufTdfError::Crypto(format!("read-back of {} failed: {e}", dest.display()))
        })?;
    Ok(())
}

fn staging_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(".partial");
    PathBuf::from(name)
}

#[allow(clippy::too_many_arguments)]
fn write_archive(
    file: &mut BufReader<File>,
    dest: &Path,
    plan: &[crate::pack::PlannedSegment],
    index: &opentdf::GgufIndex,
    payload_key: &[u8; 32],
    wrapped: &crate::key::WrappedKey,
    encryption: &TdfEncryption,
    opts: &ProtectOptions,
    virtual_size: u64,
    header_bytes: u64,
) -> Result<ProtectReport, GgufTdfError> {
    let mut builder = TdfMultiEntryBuilder::new(dest)?;

    let mut rows = Vec::with_capacity(plan.len());
    let mut tags = Vec::with_capacity(plan.len());
    let scratch_len = plan.iter().map(|s| s.plain()).max().unwrap_or(0) as usize;
    let mut scratch = Zeroizing::new(vec![0u8; scratch_len]);

    for segment in plan {
        let len = segment.plain() as usize;
        let plaintext = &mut scratch[..len];
        file.seek(SeekFrom::Start(segment.start))?;
        file.read_exact(plaintext)?;

        let encrypted = encryption
            .encrypt_segment(plaintext)
            .map_err(|e| GgufTdfError::Crypto(format!("segment encrypt failed: {e}")))?;

        builder.add_member(&segment.entry(), &encrypted.bytes)?;
        rows.push(Segment {
            hash: base64_standard(&encrypted.tag),
            segment_size: Some(segment.plain()),
            encrypted_segment_size: Some(segment.plain() + SEGMENT_OVERHEAD),
        });
        tags.push(encrypted.tag.to_vec());
    }
    drop(scratch);

    let manifest = build_manifest(payload_key, wrapped, index.clone(), rows, &tags, opts)?;
    let archive_bytes = builder.finish_with_manifest(MANIFEST_ENTRY, &manifest)?;

    Ok(ProtectReport {
        segments: plan.len(),
        virtual_size,
        header_bytes,
        archive_bytes,
        // Set by `protect` once the rename has happened and the read-back
        // has actually run; this function never sees the final path.
        verified_header: false,
    })
}

/// Assembles the OpenTDF manifest plus the `gguf` index.
fn build_manifest(
    payload_key: &[u8; 32],
    wrapped: &crate::key::WrappedKey,
    index: opentdf::GgufIndex,
    rows: Vec<Segment>,
    tags: &[Vec<u8>],
    opts: &ProtectOptions,
) -> Result<TdfManifest, GgufTdfError> {
    let mut manifest = TdfManifest::new(crate::HEADER_ENTRY.to_string(), wrapped.kas_url.clone());

    manifest.payload.mime_type = Some(opts.mime_type.clone());
    manifest.payload.tdf_spec_version = Some(TDF_SPEC_VERSION.to_string());
    manifest.tdf_spec_version = Some(TDF_SPEC_VERSION.to_string());
    manifest.schema_version = Some(TDF_SPEC_VERSION.to_string());

    let integrity = &mut manifest.encryption_information.integrity_information;
    integrity.segment_size_default = index.max_segment;
    integrity.encrypted_segment_size_default = index.max_segment + SEGMENT_OVERHEAD;
    integrity.segments = rows;
    integrity
        .generate_root_signature(tags, payload_key)
        .map_err(GgufTdfError::BadIndex)?;

    let policy_json = policy_document(&opts.attributes, &opts.dissem)?;
    manifest.set_policy_raw(&policy_json);

    let key_access = manifest
        .encryption_information
        .key_access
        .first_mut()
        .ok_or_else(|| GgufTdfError::BadIndex("manifest has no keyAccess entry".to_string()))?;
    key_access.wrapped_key.clone_from(&wrapped.wrapped_key);
    key_access.kid.clone_from(&wrapped.kid);
    key_access
        .generate_policy_binding_raw(&policy_json, payload_key)
        .map_err(GgufTdfError::BadIndex)?;

    manifest.gguf = Some(index);
    Ok(manifest)
}

/// Builds the OpenTDF Policy Object this archive is bound to.
fn policy_document(attributes: &[String], dissem: &[String]) -> Result<String, GgufTdfError> {
    let policy = serde_json::json!({
        "uuid": uuid::Uuid::new_v4().to_string(),
        "body": {
            "dataAttributes": attributes
                .iter()
                .map(|a| serde_json::json!({ "attribute": a }))
                .collect::<Vec<_>>(),
            "dissem": dissem,
        }
    });
    serde_json::to_string(&policy)
        .map_err(|e| GgufTdfError::BadIndex(format!("cannot serialize policy: {e}")))
}

fn base64_standard(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::WrappedKey;
    use opentdf::TdfMemberIndex;
    use std::io::Write;

    /// Records the payload key `protect` generated, standing in for a KAS.
    struct MockWrapper {
        captured: std::sync::Mutex<Option<[u8; 32]>>,
    }

    impl MockWrapper {
        fn new() -> Self {
            Self {
                captured: std::sync::Mutex::new(None),
            }
        }
        fn key(&self) -> [u8; 32] {
            self.captured.lock().unwrap().expect("wrap was called")
        }
    }

    impl PayloadKeyWrapper for MockWrapper {
        fn wrap(&self, payload_key: &[u8; 32]) -> Result<WrappedKey, GgufTdfError> {
            *self.captured.lock().unwrap() = Some(*payload_key);
            Ok(WrappedKey {
                kas_url: "https://kas.example.invalid".to_string(),
                kid: Some("kas-key-1".to_string()),
                wrapped_key: String::new(),
            })
        }
    }

    /// The smallest header `parse_header` accepts: magic, version 3, zero
    /// tensors, zero KV, padded to the default 32-byte alignment.
    fn minimal_gguf() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"GGUF");
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
        out.extend_from_slice(&0u64.to_le_bytes()); // kv_count
        out.resize(32, 0);
        out
    }

    /// Flips one ciphertext byte inside zip member `entry`, past its 12-byte
    /// IV — the same technique `tests/roundtrip.rs`'s `corrupt_member` uses.
    fn flip_a_ciphertext_byte(archive: &Path, entry: &str) {
        let data_start = {
            let mut file = File::open(archive).unwrap();
            let members = TdfMemberIndex::open(&mut file).unwrap();
            members.get(entry).unwrap().data_start
        };
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(archive)
            .unwrap();
        file.seek(SeekFrom::Start(data_start + 20)).unwrap();
        let mut byte = [0u8; 1];
        file.read_exact(&mut byte).unwrap();
        byte[0] ^= 0x01;
        file.seek(SeekFrom::Start(data_start + 20)).unwrap();
        file.write_all(&byte).unwrap();
    }

    #[test]
    fn read_back_fails_closed_when_the_header_member_is_corrupted() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("model.gguf");
        std::fs::write(&source, minimal_gguf()).unwrap();
        let dest = dir.path().join("model.gguf.tdf");

        let wrapper = MockWrapper::new();
        protect(&source, &dest, &wrapper, &ProtectOptions::default()).unwrap();
        let payload_key = wrapper.key();

        flip_a_ciphertext_byte(&dest, crate::HEADER_ENTRY);

        let err = read_back(&dest, &payload_key).unwrap_err();
        assert!(
            matches!(err, GgufTdfError::Crypto(_)),
            "a corrupted header member must fail read-back as Crypto, got: {err:?}"
        );
    }
}
