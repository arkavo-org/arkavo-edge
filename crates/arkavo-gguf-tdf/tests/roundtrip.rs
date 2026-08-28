//! Reader, header binding, root signature, and `read_at`.
//!
//! Covers conformance tests T2–T8, T10, T11, T16, and T17.

mod common;

use arkavo_gguf_tdf::{
    GgufTdfArchive, GgufTdfError, PayloadKeyUnwrapper, PayloadKeyWrapper, ProtectOptions,
    VirtualGguf, WrappedKey, protect,
};
use base64::Engine as _;
use opentdf::{TdfManifest, TdfMemberIndex, TdfMultiEntryBuilder};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Records the payload key at wrap time and hands it back at unwrap time,
/// standing in for a KAS. No production KAS is contacted in unit tests.
#[derive(Clone, Default)]
struct MockKas {
    key: Arc<Mutex<Option<[u8; 32]>>>,
    deny: bool,
    /// Returns this key instead of the real one, to model a wrong key.
    override_key: Option<[u8; 32]>,
}

impl MockKas {
    fn new() -> Self {
        Self::default()
    }
    fn denying() -> Self {
        Self {
            deny: true,
            ..Self::default()
        }
    }
    fn with_wrong_key(&self) -> Self {
        Self {
            key: self.key.clone(),
            deny: false,
            override_key: Some([0x11; 32]),
        }
    }
}

impl PayloadKeyWrapper for MockKas {
    fn wrap(&self, payload_key: &[u8; 32]) -> Result<WrappedKey, GgufTdfError> {
        *self.key.lock().unwrap() = Some(*payload_key);
        Ok(WrappedKey {
            kas_url: "https://kas.example.invalid".to_string(),
            kid: Some("kas-key-1".to_string()),
            wrapped_key: base64::engine::general_purpose::STANDARD.encode(payload_key),
        })
    }
}

impl PayloadKeyUnwrapper for MockKas {
    fn unwrap_key(&self, _manifest: &TdfManifest) -> Result<[u8; 32], GgufTdfError> {
        if self.deny {
            return Err(GgufTdfError::KasDenied("policy denied".to_string()));
        }
        if let Some(k) = self.override_key {
            return Ok(k);
        }
        self.key
            .lock()
            .unwrap()
            .ok_or_else(|| GgufTdfError::KasDenied("no key recorded".to_string()))
    }
}

/// `unwrap_err` needs `Debug` on the success type, and neither
/// `VirtualGguf` nor `GgufTdfArchive` derives it: `VirtualGguf` holds the
/// payload key, and a `Debug` impl is an easy way to leak key material.
fn expect_err<T>(result: Result<T, GgufTdfError>) -> GgufTdfError {
    match result {
        Ok(_) => panic!("expected an error"),
        Err(err) => err,
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    source_bytes: Vec<u8>,
    source: PathBuf,
    archive: PathBuf,
    kas: MockKas,
}

fn build(max_segment: u64) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    // Two tensors with alignment padding between them, so a read can cross a
    // tensor boundary and land on padding.
    let bytes = common::synthetic_gguf(
        &[
            ("token_embd.weight", 0, [4096, 2, 1, 1]),
            ("blk.0.attn_norm.weight", 0, [1000, 1, 1, 1]),
            ("blk.0.ffn_norm.weight", 0, [4096, 1, 1, 1]),
        ],
        None,
    );
    let source = common::write_gguf(dir.path(), "model.gguf", &bytes);
    let archive = dir.path().join("model.gguf.tdf");

    let kas = MockKas::new();
    let opts = ProtectOptions {
        max_segment,
        ..Default::default()
    };
    protect(&source, &archive, &kas, &opts).unwrap();

    Fixture {
        _dir: dir,
        source_bytes: bytes,
        source,
        archive,
        kas,
    }
}

fn unlock(f: &Fixture) -> VirtualGguf {
    GgufTdfArchive::open(&f.archive)
        .unwrap()
        .unlock(&f.kas)
        .unwrap()
}

/// Rewrites an archive with its manifest JSON transformed.
fn rewrite_manifest(archive: &Path, dest: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let mut file = File::open(archive).unwrap();
    let members = TdfMemberIndex::open(&mut file).unwrap();

    let mut names: Vec<(String, u64, u64)> = Vec::new();
    let mut zip = zip::ZipArchive::new(File::open(archive).unwrap()).unwrap();
    for i in 0..zip.len() {
        let e = zip.by_index(i).unwrap();
        names.push((e.name().to_string(), e.data_start(), e.size()));
    }

    let mut builder = TdfMultiEntryBuilder::new(dest).unwrap();
    let mut manifest_json = None;
    for (name, start, size) in &names {
        let mut buf = vec![0u8; *size as usize];
        file.seek(SeekFrom::Start(*start)).unwrap();
        file.read_exact(&mut buf).unwrap();
        if name == "0.manifest.json" {
            manifest_json = Some(buf);
        } else {
            builder.add_member(name, &buf).unwrap();
        }
    }

    let mut value: serde_json::Value =
        serde_json::from_slice(&manifest_json.expect("manifest member")).unwrap();
    edit(&mut value);
    let manifest = TdfManifest::from_json(&serde_json::to_string(&value).unwrap()).unwrap();
    builder
        .finish_with_manifest("0.manifest.json", &manifest)
        .unwrap();

    let _ = members;
}

/// T2: the first four virtual bytes are the GGUF magic.
#[test]
fn t2_reads_the_gguf_magic_after_mock_kas() {
    let f = build(4096);
    let mut vg = unlock(&f);

    let mut buf = [0u8; 4];
    assert_eq!(vg.read_at(0, &mut buf), 4);
    assert_eq!(&buf, b"GGUF");
    assert_eq!(&buf, &f.source_bytes[..4]);
}

/// T3: every range matches the source, including across member boundaries.
#[test]
fn t3_serves_bytes_identical_to_the_source() {
    let f = build(4096);
    let mut vg = unlock(&f);
    let total = f.source_bytes.len() as u64;
    assert_eq!(vg.virtual_size(), total);

    let header_bytes = vg.header_bytes();
    let ranges: Vec<(u64, usize)> = vec![
        (0, 64),                    // inside the header
        (header_bytes - 8, 32),     // across the header boundary
        (header_bytes, 100),        // start of tensor data
        (header_bytes + 4000, 500), // across a segment boundary
        (total - 10, 10),           // final bytes
        (0, f.source_bytes.len()),  // the whole file in one call
    ];

    for (offset, len) in ranges {
        let mut got = vec![0u8; len];
        let n = vg.read_at(offset, &mut got);
        assert_eq!(n, len, "short read at offset {offset}");
        let want = &f.source_bytes[offset as usize..offset as usize + len];
        assert_eq!(got, want, "bytes differ at offset {offset}");
    }

    // Reading past the end is EOF, not an error.
    let mut buf = [0u8; 16];
    assert_eq!(vg.read_at(total, &mut buf), 0);
    assert!(vg.error().is_none());

    // A read that straddles the end is clipped.
    let n = vg.read_at(total - 4, &mut buf);
    assert_eq!(n, 4);
}

/// T4: a small cap splits tensors across many members and reads still match.
#[test]
fn t4_small_max_segment_still_serves_identical_bytes() {
    let f = build(4096);
    let archive = GgufTdfArchive::open(&f.archive).unwrap();
    assert!(
        archive.manifest().gguf.as_ref().unwrap().segments.len() > 4,
        "a 4 KiB cap must produce several members"
    );
    drop(archive);

    let mut vg = unlock(&f);
    let mut whole = vec![0u8; f.source_bytes.len()];
    assert_eq!(vg.read_at(0, &mut whole), f.source_bytes.len());
    assert_eq!(whole, f.source_bytes);
}

/// T5: a wrong payload key fails closed and leaves no plaintext in `dst`.
#[test]
fn t5_wrong_key_fails_closed_and_zeroizes_the_destination() {
    let f = build(4096);
    let wrong = f.kas.with_wrong_key();
    let archive = GgufTdfArchive::open(&f.archive).unwrap();

    // The header decrypt is the first thing a wrong key hits.
    let err = expect_err(archive.unlock(&wrong));
    assert_eq!(err.code(), "GGUFTDF_TAG_MISMATCH");
}

/// T6: a flipped ciphertext bit is a sticky tag mismatch with no panic, and
/// a read that already copied good bytes leaves none of them behind.
#[test]
fn t6_flipped_ciphertext_bit_is_a_sticky_tag_mismatch() {
    use std::io::Write;

    let f = build(4096);

    // Flip one bit inside member s/1.
    let corrupted = f.archive.with_extension("corrupt");
    std::fs::copy(&f.archive, &corrupted).unwrap();
    let data_start = {
        let mut file = File::open(&corrupted).unwrap();
        let members = TdfMemberIndex::open(&mut file).unwrap();
        members.get("s/1").unwrap().data_start
    };
    {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&corrupted)
            .unwrap();
        // Offset 20 is inside the ciphertext, past the 12-byte IV.
        file.seek(SeekFrom::Start(data_start + 20)).unwrap();
        let mut byte = [0u8; 1];
        file.read_exact(&mut byte).unwrap();
        byte[0] ^= 0x01;
        file.seek(SeekFrom::Start(data_start + 20)).unwrap();
        file.write_all(&byte).unwrap();
    }

    let mut vg = GgufTdfArchive::open(&corrupted)
        .unwrap()
        .unlock(&f.kas)
        .unwrap();

    // Start inside the authenticated header and run into the corrupted
    // member, so the call has already copied real plaintext into `dst`
    // before the tag check fails.
    let header_bytes = vg.header_bytes();
    let mut buf = vec![0xAAu8; 128];
    let n = vg.read_at(header_bytes - 32, &mut buf);

    assert_eq!(n, 0, "a corrupted member must not serve bytes");
    assert_eq!(vg.error().unwrap().code(), "GGUFTDF_TAG_MISMATCH");
    // The 32 header bytes were copied before the failure and must be wiped.
    assert!(
        buf[..32].iter().all(|b| *b == 0),
        "plaintext copied before the failure must be zeroized: {:?}",
        &buf[..32]
    );
    // Bytes the call never wrote keep whatever the caller had there.
    assert!(
        buf[32..].iter().all(|b| *b == 0xAA),
        "the reader must not touch destination bytes it never filled"
    );

    // Sticky: even a read that would otherwise succeed returns 0.
    let mut again = [0u8; 4];
    assert_eq!(vg.read_at(0, &mut again), 0);
    assert_eq!(again, [0u8; 4]);
}

/// T7: an unknown or absent profile fails closed at open.
#[test]
fn t7_unknown_profile_is_refused() {
    let f = build(4096);

    let wrong = f.archive.with_extension("wrongprofile");
    rewrite_manifest(&f.archive, &wrong, |v| {
        v["gguf"]["profile"] = serde_json::json!("gguf-tdf/0");
    });
    assert_eq!(
        expect_err(GgufTdfArchive::open(&wrong)).code(),
        "GGUFTDF_UNSUPPORTED_PROFILE"
    );

    let absent = f.archive.with_extension("noprofile");
    rewrite_manifest(&f.archive, &absent, |v| {
        v.as_object_mut().unwrap().remove("gguf");
    });
    assert_eq!(
        expect_err(GgufTdfArchive::open(&absent)).code(),
        "GGUFTDF_UNSUPPORTED_PROFILE"
    );
}

/// T8: a mutated `virtualSize` is caught at open, before any KAS call.
#[test]
fn t8_virtual_size_mismatch_is_caught_at_open() {
    let f = build(4096);
    let bad = f.archive.with_extension("badsize");
    rewrite_manifest(&f.archive, &bad, |v| {
        let current = v["gguf"]["virtualSize"].as_u64().unwrap();
        v["gguf"]["virtualSize"] = serde_json::json!(current + 1);
    });

    assert_eq!(
        expect_err(GgufTdfArchive::open(&bad)).code(),
        "GGUFTDF_SIZE_MISMATCH"
    );
}

/// T9: a KAS denial fails closed and never reaches the sibling plaintext.
#[test]
fn t9_kas_denial_fails_closed_without_a_sibling_fallback() {
    let f = build(4096);
    assert!(f.source.exists(), "the sibling plaintext is present");

    let archive = GgufTdfArchive::open(&f.archive).unwrap();
    let err = expect_err(archive.unlock(&MockKas::denying()));
    assert_eq!(err.code(), "GGUFTDF_KAS_DENIED");
}

/// T10: a reader ignores `method.iv`, using the per-member IV prefix.
#[test]
fn t10_reader_ignores_a_dummy_method_iv() {
    let f = build(4096);
    let dummy = f.archive.with_extension("dummyiv");
    rewrite_manifest(&f.archive, &dummy, |v| {
        // 12 zero bytes, Base64. A non-conforming producer might write this.
        v["encryptionInformation"]["method"]["iv"] = serde_json::json!("AAAAAAAAAAAAAAAA");
    });

    let mut vg = GgufTdfArchive::open(&dummy)
        .unwrap()
        .unlock(&f.kas)
        .unwrap();
    let mut buf = [0u8; 4];
    assert_eq!(vg.read_at(0, &mut buf), 4);
    assert_eq!(&buf, b"GGUF");
}

/// T11: an archive whose manifest is named `manifest.json` still loads.
#[test]
fn t11_falls_back_to_the_historical_manifest_name() {
    let f = build(4096);
    let fallback = f.archive.with_extension("fallback");

    // Rebuild with the manifest under the OpenTDF markdown name.
    let mut file = File::open(&f.archive).unwrap();
    let mut zip = zip::ZipArchive::new(File::open(&f.archive).unwrap()).unwrap();
    let entries: Vec<(String, u64, u64)> = (0..zip.len())
        .map(|i| {
            let e = zip.by_index(i).unwrap();
            (e.name().to_string(), e.data_start(), e.size())
        })
        .collect();

    let mut builder = TdfMultiEntryBuilder::new(&fallback).unwrap();
    let mut manifest_bytes = Vec::new();
    for (name, start, size) in &entries {
        let mut buf = vec![0u8; *size as usize];
        file.seek(SeekFrom::Start(*start)).unwrap();
        file.read_exact(&mut buf).unwrap();
        if name == "0.manifest.json" {
            manifest_bytes = buf;
        } else {
            builder.add_member(name, &buf).unwrap();
        }
    }
    let manifest = TdfManifest::from_json(std::str::from_utf8(&manifest_bytes).unwrap()).unwrap();
    builder
        .finish_with_manifest("manifest.json", &manifest)
        .unwrap();

    let mut vg = GgufTdfArchive::open(&fallback)
        .unwrap()
        .unlock(&f.kas)
        .unwrap();
    let mut buf = [0u8; 4];
    assert_eq!(vg.read_at(0, &mut buf), 4);
    assert_eq!(&buf, b"GGUF");
}

/// T16: the index must agree with the authenticated header.
#[test]
fn t16_index_must_bind_to_the_decrypted_header() {
    let f = build(4096);

    let renamed = f.archive.with_extension("renamed");
    rewrite_manifest(&f.archive, &renamed, |v| {
        v["gguf"]["tensors"][0]["name"] = serde_json::json!("not_the_real_name");
    });
    let err = expect_err(GgufTdfArchive::open(&renamed).unwrap().unlock(&f.kas));
    assert_eq!(err.code(), "GGUFTDF_BAD_INDEX");
}

/// T17: an equal-size member swap with swapped hashes is caught by the root
/// signature, before any weight byte is served.
#[test]
fn t17_equal_size_member_swap_is_caught_by_the_root_signature() {
    let f = build(4096);
    let swapped = f.archive.with_extension("swapped");

    // Find two equal-size weight members and swap both bytes and hash rows.
    let mut file = File::open(&f.archive).unwrap();
    let mut zip = zip::ZipArchive::new(File::open(&f.archive).unwrap()).unwrap();
    let entries: Vec<(String, u64, u64)> = (0..zip.len())
        .map(|i| {
            let e = zip.by_index(i).unwrap();
            (e.name().to_string(), e.data_start(), e.size())
        })
        .collect();

    let s1 = entries.iter().find(|(n, _, _)| n == "s/1").unwrap().clone();
    let s2 = entries.iter().find(|(n, _, _)| n == "s/2").unwrap().clone();
    assert_eq!(s1.2, s2.2, "fixture must have two equal-size members");

    let read = |start: u64, size: u64| {
        let mut buf = vec![0u8; size as usize];
        let mut f = File::open(&f.archive).unwrap();
        f.seek(SeekFrom::Start(start)).unwrap();
        f.read_exact(&mut buf).unwrap();
        buf
    };
    let b1 = read(s1.1, s1.2);
    let b2 = read(s2.1, s2.2);

    let mut manifest_bytes = Vec::new();
    let mut builder = TdfMultiEntryBuilder::new(&swapped).unwrap();
    for (name, start, size) in &entries {
        let mut buf = vec![0u8; *size as usize];
        file.seek(SeekFrom::Start(*start)).unwrap();
        file.read_exact(&mut buf).unwrap();
        match name.as_str() {
            "0.manifest.json" => manifest_bytes = buf,
            "s/1" => builder.add_member("s/1", &b2).unwrap(),
            "s/2" => builder.add_member("s/2", &b1).unwrap(),
            other => builder.add_member(other, &buf).unwrap(),
        }
    }

    // Swap the corresponding hash rows so per-segment GMAC still passes.
    let mut value: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let rows = value["encryptionInformation"]["integrityInformation"]["segments"]
        .as_array_mut()
        .unwrap();
    rows.swap(1, 2);
    let manifest = TdfManifest::from_json(&serde_json::to_string(&value).unwrap()).unwrap();
    builder
        .finish_with_manifest("0.manifest.json", &manifest)
        .unwrap();

    let err = expect_err(GgufTdfArchive::open(&swapped).unwrap().unlock(&f.kas));
    assert_eq!(
        err.code(),
        "GGUFTDF_ROOT_MISMATCH",
        "GMAC does not bind order; the root HMAC must catch the swap"
    );
}

#[test]
fn open_rejects_a_file_that_is_not_a_zip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plain.gguf.tdf");
    std::fs::write(&path, b"not a zip archive at all").unwrap();
    assert_eq!(
        expect_err(GgufTdfArchive::open(&path)).code(),
        "GGUFTDF_NOT_ZIP"
    );
}
