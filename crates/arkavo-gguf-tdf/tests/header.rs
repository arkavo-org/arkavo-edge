//! GGUF header parsing (spec §7): geometry, alignment, and the §7.3
//! magic/version/endianness gate.

mod common;
use common as fixture;

use arkavo_gguf_tdf::{identify, parse_header};
use std::io::Cursor;

#[test]
fn parses_alignment_data_offset_and_tensor_geometry() {
    // 4096x2 F32 (32768 B) then 4096x1 F32 (16384 B), default ALIGN 32.
    let bytes = fixture::synthetic_gguf(
        &[
            ("token_embd.weight", 0, [4096, 2, 1, 1]),
            ("blk.0.attn_norm.weight", 0, [4096, 1, 1, 1]),
        ],
        None,
    );

    let header = parse_header(&mut Cursor::new(&bytes)).unwrap();
    assert_eq!(header.alignment, 32);
    assert!(header.data_offset > 0);
    assert!(header.data_offset.is_multiple_of(32));
    assert_eq!(header.tensors.len(), 2);

    assert_eq!(header.tensors[0].name, "token_embd.weight");
    assert_eq!(header.tensors[0].gguf_offset, 0);
    assert_eq!(header.tensors[0].size, 4096 * 2 * 4);

    assert_eq!(header.tensors[1].name, "blk.0.attn_norm.weight");
    assert_eq!(header.tensors[1].gguf_offset, 32768);
    assert_eq!(header.tensors[1].size, 4096 * 4);

    // The fixture's file length is exactly header + both tensors.
    assert_eq!(bytes.len() as u64, header.data_offset + 32768 + 16384);
}

#[test]
fn honours_explicit_general_alignment() {
    let bytes = fixture::synthetic_gguf(&[("a", 0, [64, 1, 1, 1])], Some(64));
    let header = parse_header(&mut Cursor::new(&bytes)).unwrap();
    assert_eq!(header.alignment, 64);
    assert!(header.data_offset.is_multiple_of(64));
}

#[test]
fn rejects_alignment_that_is_not_a_power_of_two() {
    // [GGUF] allows any multiple of 8; this profile also requires a power of
    // two so the default 4 MiB maxSegment stays a whole multiple of ALIGN.
    let bytes = fixture::synthetic_gguf(&[("a", 0, [64, 1, 1, 1])], Some(24));
    let err = parse_header(&mut Cursor::new(&bytes)).unwrap_err();
    assert_eq!(err.code(), "GGUFTDF_BAD_ALIGN");
}

#[test]
fn parses_a_header_only_file() {
    let bytes = fixture::synthetic_gguf(&[], None);
    let header = parse_header(&mut Cursor::new(&bytes)).unwrap();
    assert!(header.tensors.is_empty());
    assert_eq!(bytes.len() as u64, header.data_offset);
}

#[test]
fn identify_enforces_magic_version_and_endianness() {
    // Little-endian GGUF v3 is the only accepted shape.
    assert!(identify(&[0x47, 0x47, 0x55, 0x46, 3, 0, 0, 0]).is_ok());

    // A reversed magic is not a big-endian marker; it is simply not GGUF.
    assert_eq!(
        identify(&[0x46, 0x55, 0x47, 0x47, 0, 0, 0, 3])
            .unwrap_err()
            .code(),
        "GGUFTDF_NOT_GGUF"
    );

    // Big-endian v3 keeps the GGUF magic and moves the version bytes.
    assert_eq!(
        identify(&[0x47, 0x47, 0x55, 0x46, 0, 0, 0, 3])
            .unwrap_err()
            .code(),
        "GGUFTDF_UNSUPPORTED_ENDIAN"
    );

    // v1 and v2 are refused as versions, not as endianness.
    for v in [1u32, 2] {
        let mut buf = vec![0x47, 0x47, 0x55, 0x46];
        buf.extend_from_slice(&v.to_le_bytes());
        assert_eq!(
            identify(&buf).unwrap_err().code(),
            "GGUFTDF_UNSUPPORTED_GGUF_VERSION",
            "version {v} must be refused as a version"
        );
    }

    // Too short to hold a magic and a version.
    assert_eq!(
        identify(&[0x47, 0x47, 0x55, 0x46]).unwrap_err().code(),
        "GGUFTDF_NOT_GGUF"
    );
}

#[test]
fn rejects_tensor_name_at_or_past_ggml_max_name() {
    // ggml rejects `length >= GGML_MAX_NAME` (64), so 64 bytes must fail.
    let long = "n".repeat(64);
    let bytes = fixture::synthetic_gguf(&[(long.as_str(), 0, [32, 1, 1, 1])], None);
    let err = parse_header(&mut Cursor::new(&bytes)).unwrap_err();
    assert_eq!(err.code(), "GGUFTDF_BAD_TENSOR");

    // 63 bytes is the largest name ggml will load.
    let ok = "n".repeat(63);
    let bytes = fixture::synthetic_gguf(&[(ok.as_str(), 0, [32, 1, 1, 1])], None);
    let header = parse_header(&mut Cursor::new(&bytes)).unwrap();
    assert_eq!(header.tensors[0].name.len(), 63);
}

#[test]
fn rejects_a_truncated_header() {
    let bytes = fixture::synthetic_gguf(&[("a", 0, [64, 1, 1, 1])], None);
    let truncated = &bytes[..20];
    let err = parse_header(&mut Cursor::new(truncated)).unwrap_err();
    assert_eq!(
        err.code(),
        "GGUFTDF_BAD_INDEX",
        "truncation is an I/O error"
    );
}
