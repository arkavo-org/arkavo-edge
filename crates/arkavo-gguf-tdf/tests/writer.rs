//! Wrap procedure (spec §12) and conformance tests T1 and T18.

mod common;

use arkavo_gguf_tdf::{GgufTdfError, PayloadKeyWrapper, ProtectOptions, WrappedKey, protect};
use base64::Engine as _;
use opentdf::{TdfManifest, TdfMemberIndex};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// A wrapper that records the payload key instead of contacting a KAS.
/// Unit tests must never reach a production KAS.
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
            // Stands in for an RSA-OAEP wrap; the reader side is mocked too.
            wrapped_key: base64::engine::general_purpose::STANDARD.encode(payload_key),
        })
    }
}

fn read_manifest(path: &std::path::Path) -> (TdfManifest, TdfMemberIndex) {
    let mut file = File::open(path).unwrap();
    let members = TdfMemberIndex::open(&mut file).unwrap();
    let loc = members.get("0.manifest.json").expect("manifest member");
    let mut json = vec![0u8; loc.size as usize];
    file.seek(SeekFrom::Start(loc.data_start)).unwrap();
    file.read_exact(&mut json).unwrap();
    (
        TdfManifest::from_json(std::str::from_utf8(&json).unwrap()).unwrap(),
        members,
    )
}

/// T1: a tiny GGUF with at least one tensor packs into the profile layout.
#[test]
fn t1_packs_a_tiny_gguf_into_profile_members() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = common::synthetic_gguf(
        &[
            ("token_embd.weight", 0, [4096, 2, 1, 1]),
            ("blk.0.attn_norm.weight", 0, [4096, 1, 1, 1]),
        ],
        None,
    );
    let source = common::write_gguf(dir.path(), "model.gguf", &bytes);
    let dest = dir.path().join("model.gguf.tdf");

    let wrapper = MockWrapper::new();
    let report = protect(&source, &dest, &wrapper, &ProtectOptions::default()).unwrap();

    assert_eq!(report.virtual_size, bytes.len() as u64);
    assert!(
        report.segments >= 2,
        "header plus at least one weight member"
    );

    let (manifest, members) = read_manifest(&dest);

    // Members: 0.manifest.json, header, s/1.., and never 0.payload.
    assert!(members.contains("header"));
    assert!(members.contains("s/1"));
    assert!(
        !members.contains("0.payload"),
        "the profile forbids a concatenated payload member"
    );

    // Payload object.
    assert_eq!(manifest.payload.url, "header");
    assert_eq!(manifest.payload.protocol, "zip");
    assert_eq!(manifest.payload.payload_type, "reference");
    assert!(manifest.payload.is_encrypted);
    assert_eq!(
        manifest.payload.mime_type.as_deref(),
        Some("application/x-gguf")
    );
    assert_eq!(manifest.payload.tdf_spec_version.as_deref(), Some("4.3.0"));
    assert_eq!(manifest.tdf_spec_version.as_deref(), Some("4.3.0"));
    assert_eq!(manifest.schema_version.as_deref(), Some("4.3.0"));

    // Method: per-member IVs, so method.iv is the empty string.
    let enc = &manifest.encryption_information;
    assert_eq!(enc.method.algorithm, "AES-256-GCM");
    assert!(enc.method.is_streamable);
    assert_eq!(enc.method.iv, "", "method.iv must be empty, not a dummy IV");

    // Integrity rows line up with the index and the zip.
    let integrity = &enc.integrity_information;
    assert_eq!(integrity.segment_hash_alg, "GMAC");
    assert_eq!(integrity.root_signature.alg, "HS256");
    let sig = base64::engine::general_purpose::STANDARD
        .decode(&integrity.root_signature.sig)
        .unwrap();
    assert_eq!(
        sig.len(),
        32,
        "root signature is Base64 of the raw 32-byte MAC"
    );

    let index = manifest.gguf.as_ref().expect("gguf index");
    assert_eq!(index.profile, "gguf-tdf/1");
    assert_eq!(index.virtual_size, bytes.len() as u64);
    assert_eq!(index.tensors.len(), 2);
    assert_eq!(index.segments.len(), integrity.segments.len());

    for (seg, row) in index.segments.iter().zip(&integrity.segments) {
        let plain = row.segment_size.unwrap();
        assert_eq!(plain, seg.plain);
        assert_eq!(row.encrypted_segment_size.unwrap(), plain + 28);
        let member = members.get(&seg.entry).expect("member for every segment");
        assert_eq!(member.size, plain + 28);
        let tag = base64::engine::general_purpose::STANDARD
            .decode(&row.hash)
            .unwrap();
        assert_eq!(tag.len(), 16, "GMAC hash is the raw 16-byte GCM tag");
    }

    // keyAccess: one wrapped entry, no sid, an 88-character hex-then-base64
    // policy binding, which is what the platform KAS verifies.
    assert_eq!(enc.key_access.len(), 1);
    let ka = &enc.key_access[0];
    assert_eq!(ka.access_type, "wrapped");
    assert_eq!(ka.protocol, "kas");
    assert_eq!(ka.kid.as_deref(), Some("kas-key-1"));
    assert_eq!(ka.policy_binding.alg, "HS256");
    let binding = base64::engine::general_purpose::STANDARD
        .decode(&ka.policy_binding.hash)
        .unwrap();
    assert_eq!(
        binding.len(),
        64,
        "binding is Base64 of 64 hex characters, not of the 32 raw MAC bytes"
    );
    assert!(binding.iter().all(|b| b.is_ascii_hexdigit()));

    // The manifest JSON must not carry a `sid`.
    let json = manifest.to_json().unwrap();
    assert!(!json.contains("\"sid\""), "v1 writers omit sid");

    // The source is left in place: wrapping is additive.
    assert!(source.exists());
    assert_eq!(
        std::fs::metadata(&source).unwrap().len(),
        bytes.len() as u64
    );

    // The wrapper really did see a key.
    assert_ne!(wrapper.key(), [0u8; 32]);
}

#[test]
fn manifest_is_the_last_member_so_the_root_signature_covers_every_tag() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = common::synthetic_gguf(&[("a", 0, [4096, 1, 1, 1])], None);
    let source = common::write_gguf(dir.path(), "m.gguf", &bytes);
    let dest = dir.path().join("m.gguf.tdf");
    protect(
        &source,
        &dest,
        &MockWrapper::new(),
        &ProtectOptions::default(),
    )
    .unwrap();

    let mut file = File::open(&dest).unwrap();
    let mut zip = zip::ZipArchive::new(&mut file).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    assert_eq!(names.last().unwrap(), "0.manifest.json");
    assert_eq!(names.first().unwrap(), "header");
}

#[test]
fn small_max_segment_splits_a_tensor_across_many_members() {
    let dir = tempfile::tempdir().unwrap();
    // 1 MiB of F32 weights at a 4096-byte cap.
    let bytes = common::synthetic_gguf(&[("big", 0, [262_144, 1, 1, 1])], None);
    let source = common::write_gguf(dir.path(), "big.gguf", &bytes);
    let dest = dir.path().join("big.gguf.tdf");

    let opts = ProtectOptions {
        max_segment: 4096,
        ..Default::default()
    };
    let report = protect(&source, &dest, &MockWrapper::new(), &opts).unwrap();

    assert!(
        report.segments > 250,
        "1 MiB at a 4 KiB cap needs many members, got {}",
        report.segments
    );

    let (manifest, members) = read_manifest(&dest);
    let index = manifest.gguf.unwrap();
    assert!(index.segments.iter().skip(1).all(|s| s.plain <= 4096));
    assert_eq!(
        index.segments.iter().map(|s| s.plain).sum::<u64>(),
        bytes.len() as u64
    );
    assert!(members.contains("s/250"));
}

#[test]
fn wraps_a_policy_with_the_requested_attributes() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = common::synthetic_gguf(&[("a", 0, [4096, 1, 1, 1])], None);
    let source = common::write_gguf(dir.path(), "m.gguf", &bytes);
    let dest = dir.path().join("m.gguf.tdf");

    let opts = ProtectOptions {
        attributes: vec!["https://arkavo.net/attr/data/clearance/value/internal".to_string()],
        ..Default::default()
    };
    protect(&source, &dest, &MockWrapper::new(), &opts).unwrap();

    let (manifest, _) = read_manifest(&dest);
    let policy = manifest.get_policy_raw().unwrap();
    let value: serde_json::Value = serde_json::from_str(&policy).unwrap();
    assert_eq!(
        value["body"]["dataAttributes"][0]["attribute"],
        "https://arkavo.net/attr/data/clearance/value/internal"
    );
    assert!(value["uuid"].as_str().unwrap().len() >= 36);
    assert!(value["body"]["dissem"].as_array().unwrap().is_empty());
}

/// T18: endianness and magic gate on the writer side.
#[test]
fn t18_refuses_big_endian_and_non_gguf_sources() {
    let dir = tempfile::tempdir().unwrap();
    let opts = ProtectOptions::default();

    // Magic is GGUF, version bytes say big-endian 3.
    let mut be = vec![0x47, 0x47, 0x55, 0x46, 0x00, 0x00, 0x00, 0x03];
    be.extend_from_slice(&[0u8; 64]);
    let source = common::write_gguf(dir.path(), "be.gguf", &be);
    assert_eq!(
        protect(
            &source,
            &dir.path().join("be.gguf.tdf"),
            &MockWrapper::new(),
            &opts
        )
        .unwrap_err()
        .code(),
        "GGUFTDF_UNSUPPORTED_ENDIAN"
    );

    // A reversed magic is not a big-endian marker; it is not GGUF at all.
    let mut reversed = vec![0x46, 0x55, 0x47, 0x47, 0x03, 0x00, 0x00, 0x00];
    reversed.extend_from_slice(&[0u8; 64]);
    let source = common::write_gguf(dir.path(), "rev.gguf", &reversed);
    assert_eq!(
        protect(
            &source,
            &dir.path().join("rev.gguf.tdf"),
            &MockWrapper::new(),
            &opts
        )
        .unwrap_err()
        .code(),
        "GGUFTDF_NOT_GGUF"
    );

    // GGUF v2 is refused as a version.
    let mut v2 = vec![0x47, 0x47, 0x55, 0x46, 0x02, 0x00, 0x00, 0x00];
    v2.extend_from_slice(&[0u8; 64]);
    let source = common::write_gguf(dir.path(), "v2.gguf", &v2);
    assert_eq!(
        protect(
            &source,
            &dir.path().join("v2.gguf.tdf"),
            &MockWrapper::new(),
            &opts
        )
        .unwrap_err()
        .code(),
        "GGUFTDF_UNSUPPORTED_GGUF_VERSION"
    );
}

#[test]
fn a_header_only_gguf_wraps_without_weight_members() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = common::synthetic_gguf(&[], None);
    let source = common::write_gguf(dir.path(), "vocab.gguf", &bytes);
    let dest = dir.path().join("vocab.gguf.tdf");

    let report = protect(
        &source,
        &dest,
        &MockWrapper::new(),
        &ProtectOptions::default(),
    )
    .unwrap();
    assert_eq!(report.segments, 1);
    assert_eq!(report.header_bytes, report.virtual_size);

    let (_, members) = read_manifest(&dest);
    assert!(members.contains("header"));
    assert!(!members.contains("s/1"));
}
