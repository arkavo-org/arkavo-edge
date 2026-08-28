//! Wrap procedure (spec §12).
//!
//! Each planned segment is read from the source with a seek plus one bounded
//! read, so a multi-gigabyte model is never buffered. The manifest is written
//! last because its root signature covers every segment tag.

use crate::error::GgufTdfError;
use crate::gguf_header::parse_header;
use crate::index::build_index;
use crate::key::PayloadKeyWrapper;
use crate::pack::plan_segments;
use crate::{DEFAULT_MAX_SEGMENT, MANIFEST_ENTRY, SEGMENT_OVERHEAD};
use opentdf::manifest::{IntegrityInformationExt, KeyAccessExt};
use opentdf::{Segment, TdfEncryption, TdfManifest, TdfMultiEntryBuilder};
use rand::RngCore;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
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
        .map_err(|e| GgufTdfError::BadIndex(format!("payload key rejected: {e}")))?;

    let mut builder = TdfMultiEntryBuilder::new(dest)
        .map_err(|e| GgufTdfError::BadIndex(format!("cannot create {}: {e}", dest.display())))?;

    let mut rows = Vec::with_capacity(plan.len());
    let mut tags = Vec::with_capacity(plan.len());
    // One reusable plaintext buffer, sized to the largest planned segment.
    let scratch_len = plan.iter().map(|s| s.plain()).max().unwrap_or(0) as usize;
    let mut scratch = Zeroizing::new(vec![0u8; scratch_len]);

    for segment in &plan {
        let len = segment.plain() as usize;
        let plaintext = &mut scratch[..len];
        file.seek(SeekFrom::Start(segment.start))?;
        file.read_exact(plaintext)?;

        let encrypted = encryption
            .encrypt_segment(plaintext)
            .map_err(|e| GgufTdfError::BadIndex(format!("segment encrypt failed: {e}")))?;

        builder.add_member(&segment.entry(), &encrypted.bytes)?;
        rows.push(Segment {
            hash: base64_standard(&encrypted.tag),
            segment_size: Some(segment.plain()),
            encrypted_segment_size: Some(segment.plain() + SEGMENT_OVERHEAD),
        });
        tags.push(encrypted.tag.to_vec());
    }
    drop(scratch);

    let manifest = build_manifest(&payload_key, &wrapped, index, rows, &tags, opts)?;
    let archive_bytes = builder.finish_with_manifest(MANIFEST_ENTRY, &manifest)?;

    Ok(ProtectReport {
        segments: plan.len(),
        virtual_size,
        header_bytes: header.data_offset,
        archive_bytes,
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
