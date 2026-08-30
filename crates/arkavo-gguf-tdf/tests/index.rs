//! Hybrid index construction and the §9.4 open-time invariants.

use arkavo_gguf_tdf::{
    GgufHeader, HeaderTensor, MAX_HEADER_BYTES, MAX_MAX_SEGMENT, SegmentMap, build_index,
    plan_segments, validate_index,
};
use opentdf::{
    GgufIndex, GgufSegmentKind, IntegrityInformation, RootSignature, Segment, TdfManifest,
    TdfMemberIndex, TdfMultiEntryBuilder,
};

fn header(alignment: u64, data_offset: u64, tensors: Vec<HeaderTensor>) -> GgufHeader {
    GgufHeader {
        alignment,
        data_offset,
        tensors,
    }
}

fn tensor(name: &str, gguf_offset: u64, size: u64) -> HeaderTensor {
    HeaderTensor {
        name: name.to_string(),
        gguf_offset,
        size,
    }
}

/// The Appendix A source: ALIGN 32, headerBytes 64, maxSegment 128.
fn appendix_a_index() -> GgufIndex {
    let h = header(
        32,
        64,
        vec![
            tensor("token_embd.weight", 0, 256),
            tensor("blk.0.attn_norm.weight", 256, 32),
        ],
    );
    let plan = plan_segments(&h, 352, 128).unwrap();
    build_index(&h, &plan, 352, 128).unwrap()
}

/// Integrity rows matching an index, with placeholder hashes.
fn integrity_for(index: &GgufIndex) -> IntegrityInformation {
    IntegrityInformation {
        root_signature: RootSignature {
            alg: "HS256".to_string(),
            sig: String::new(),
        },
        segment_hash_alg: "GMAC".to_string(),
        segments: index
            .segments
            .iter()
            .map(|s| Segment {
                hash: String::new(),
                segment_size: Some(s.plain),
                encrypted_segment_size: Some(s.plain + 28),
            })
            .collect(),
        segment_size_default: index.max_segment,
        encrypted_segment_size_default: index.max_segment + 28,
    }
}

/// A zip whose members match the index's declared encrypted sizes.
fn members_for(dir: &std::path::Path, index: &GgufIndex) -> TdfMemberIndex {
    let path = dir.join("m.gguf.tdf");
    let mut builder = TdfMultiEntryBuilder::new(&path).unwrap();
    for seg in &index.segments {
        builder
            .add_member(&seg.entry, &vec![0u8; (seg.plain + 28) as usize])
            .unwrap();
    }
    let manifest = TdfManifest::new("header".to_string(), "https://kas.invalid".to_string());
    builder
        .finish_with_manifest("0.manifest.json", &manifest)
        .unwrap();

    TdfMemberIndex::open(std::fs::File::open(&path).unwrap()).unwrap()
}

#[test]
fn appendix_a_index_matches_the_published_vector() {
    let index = appendix_a_index();

    assert_eq!(index.profile, "gguf-tdf/1");
    assert_eq!(index.alignment, 32);
    assert_eq!(index.header_bytes, 64);
    assert_eq!(index.virtual_size, 352);
    assert_eq!(index.max_segment, 128);

    let segments: Vec<_> = index
        .segments
        .iter()
        .map(|s| (s.id, s.kind, s.plain, s.entry.as_str()))
        .collect();
    assert_eq!(
        segments,
        vec![
            (0, GgufSegmentKind::Header, 64, "header"),
            (1, GgufSegmentKind::Tensor, 128, "s/1"),
            (2, GgufSegmentKind::Tensor, 128, "s/2"),
            (3, GgufSegmentKind::Tensor, 32, "s/3"),
        ]
    );

    let tensors: Vec<_> = index
        .tensors
        .iter()
        .map(|t| (t.name.as_str(), t.offset, t.size, t.segments))
        .collect();
    assert_eq!(
        tensors,
        vec![
            ("token_embd.weight", 64, 256, [1, 3]),
            ("blk.0.attn_norm.weight", 320, 32, [3, 4]),
        ],
        "half-open ranges must match schemas/gguf-tdf/draft-00/appendix-a-packing-plan.json"
    );
}

#[test]
fn segment_map_locates_offsets_at_and_around_boundaries() {
    let index = appendix_a_index();
    let map = SegmentMap::new(&index);

    assert_eq!(map.len(), 4);
    assert_eq!(map.virtual_size(), 352);

    assert_eq!(map.covering(0), Some(0));
    assert_eq!(map.covering(63), Some(0));
    assert_eq!(map.covering(64), Some(1), "first byte of s/1");
    assert_eq!(map.covering(191), Some(1));
    assert_eq!(map.covering(192), Some(2), "first byte of s/2");
    assert_eq!(map.covering(319), Some(2));
    assert_eq!(map.covering(320), Some(3));
    assert_eq!(map.covering(351), Some(3));
    assert_eq!(map.covering(352), None, "one past the end");
    assert_eq!(map.covering(u64::MAX), None);

    assert_eq!(map.start_of(0), 0);
    assert_eq!(map.start_of(1), 64);
    assert_eq!(map.start_of(3), 320);
}

#[test]
fn a_well_formed_index_validates() {
    let dir = tempfile::tempdir().unwrap();
    let index = appendix_a_index();
    let integrity = integrity_for(&index);
    let members = members_for(dir.path(), &index);

    let map = validate_index(&index, &integrity, &members).unwrap();
    assert_eq!(map.virtual_size(), 352);
}

#[test]
fn rejects_virtual_size_that_disagrees_with_the_segment_sum() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = appendix_a_index();
    let integrity = integrity_for(&index);
    let members = members_for(dir.path(), &index);

    index.virtual_size = 353;
    assert_eq!(
        validate_index(&index, &integrity, &members)
            .unwrap_err()
            .code(),
        "GGUFTDF_SIZE_MISMATCH"
    );
}

#[test]
fn rejects_a_segment_missing_from_the_zip() {
    let dir = tempfile::tempdir().unwrap();
    let index = appendix_a_index();
    let integrity = integrity_for(&index);

    // Build a zip that omits s/2.
    let path = dir.path().join("short.gguf.tdf");
    let mut builder = TdfMultiEntryBuilder::new(&path).unwrap();
    for seg in index.segments.iter().filter(|s| s.entry != "s/2") {
        builder
            .add_member(&seg.entry, &vec![0u8; (seg.plain + 28) as usize])
            .unwrap();
    }
    let manifest = TdfManifest::new("header".to_string(), "https://kas.invalid".to_string());
    builder
        .finish_with_manifest("0.manifest.json", &manifest)
        .unwrap();
    let members = TdfMemberIndex::open(std::fs::File::open(&path).unwrap()).unwrap();

    assert_eq!(
        validate_index(&index, &integrity, &members)
            .unwrap_err()
            .code(),
        "GGUFTDF_BAD_INDEX"
    );
}

#[test]
fn rejects_encrypted_size_that_is_not_plain_plus_overhead() {
    let dir = tempfile::tempdir().unwrap();
    let index = appendix_a_index();
    let mut integrity = integrity_for(&index);
    let members = members_for(dir.path(), &index);

    integrity.segments[1].encrypted_segment_size = Some(index.segments[1].plain + 27);
    assert_eq!(
        validate_index(&index, &integrity, &members)
            .unwrap_err()
            .code(),
        "GGUFTDF_BAD_INDEX"
    );
}

#[test]
fn rejects_a_non_header_segment_past_max_segment() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = appendix_a_index();
    index.segments[1].plain = 256;
    index.virtual_size = 480;
    let integrity = integrity_for(&index);
    let members = members_for(dir.path(), &index);

    assert_eq!(
        validate_index(&index, &integrity, &members)
            .unwrap_err()
            .code(),
        "GGUFTDF_BAD_INDEX"
    );
}

#[test]
fn header_segment_may_exceed_max_segment() {
    let dir = tempfile::tempdir().unwrap();
    let h = header(32, 8192, vec![tensor("a", 0, 128)]);
    let plan = plan_segments(&h, 8192 + 128, 128).unwrap();
    let index = build_index(&h, &plan, 8192 + 128, 128).unwrap();
    let integrity = integrity_for(&index);
    let members = members_for(dir.path(), &index);

    assert_eq!(index.segments[0].plain, 8192);
    validate_index(&index, &integrity, &members).expect("header may exceed the cap");
}

#[test]
fn rejects_a_wrong_profile_string() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = appendix_a_index();
    let integrity = integrity_for(&index);
    let members = members_for(dir.path(), &index);

    index.profile = "gguf-tdf/0".to_string();
    assert_eq!(
        validate_index(&index, &integrity, &members)
            .unwrap_err()
            .code(),
        "GGUFTDF_UNSUPPORTED_PROFILE"
    );
}

#[test]
fn rejects_a_tensor_whose_declared_segment_range_is_wrong() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = appendix_a_index();
    let integrity = integrity_for(&index);
    let members = members_for(dir.path(), &index);

    // Point the first tensor at the header segment.
    index.tensors[0].segments = [0, 3];
    assert_eq!(
        validate_index(&index, &integrity, &members)
            .unwrap_err()
            .code(),
        "GGUFTDF_BAD_INDEX"
    );
}

#[test]
fn rejects_duplicate_tensor_names() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = appendix_a_index();
    let integrity = integrity_for(&index);
    let members = members_for(dir.path(), &index);

    index.tensors[1].name = index.tensors[0].name.clone();
    assert_eq!(
        validate_index(&index, &integrity, &members)
            .unwrap_err()
            .code(),
        "GGUFTDF_BAD_INDEX"
    );
}

// Task 10: absurd `headerBytes` / `maxSegment` are refused before anything
// downstream allocates a buffer sized from them.

#[test]
fn rejects_max_segment_over_the_cap() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = appendix_a_index();
    let integrity = integrity_for(&index);
    let members = members_for(dir.path(), &index);

    // Stays a multiple of the fixture's 32-byte alignment.
    index.max_segment = MAX_MAX_SEGMENT + 32;
    assert_eq!(
        validate_index(&index, &integrity, &members)
            .unwrap_err()
            .code(),
        "GGUFTDF_BAD_MAX_SEGMENT"
    );
}

/// `members` is built from the *pristine* index, so the real "header" zip
/// member stays a few bytes even though `index.header_bytes` is mutated to
/// exceed the cap afterward — the point of the cap is exactly that no test
/// (and no reader) has to construct a real ~1 GiB member to prove this.
///
/// `virtual_size` and `segments[0].plain` are raised to match, so that
/// *without* the cap this reaches invariant 5 (the member's real on-disk
/// size disagreeing with the manifest's claim) rather than an earlier,
/// unrelated check — proving the new cap, not a pre-existing one, is what
/// now catches it first.
#[test]
fn rejects_header_bytes_over_the_cap() {
    let dir = tempfile::tempdir().unwrap();
    let pristine = appendix_a_index();
    let members = members_for(dir.path(), &pristine);

    let mut index = pristine;
    let huge = MAX_HEADER_BYTES + 32;
    index.header_bytes = huge;
    index.segments[0].plain = huge;
    index.virtual_size = huge;
    let integrity = integrity_for(&index);

    assert_eq!(
        validate_index(&index, &integrity, &members)
            .unwrap_err()
            .code(),
        "GGUFTDF_BAD_HEADER"
    );
}

#[test]
fn rejects_a_renamed_segment_entry() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = appendix_a_index();
    let integrity = integrity_for(&index);
    let members = members_for(dir.path(), &index);

    // `s/01` is not the profile's grammar even though it names the same id.
    index.segments[1].entry = "s/01".to_string();
    assert_eq!(
        validate_index(&index, &integrity, &members)
            .unwrap_err()
            .code(),
        "GGUFTDF_BAD_INDEX"
    );
}
