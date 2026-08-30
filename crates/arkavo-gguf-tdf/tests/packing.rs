//! Segment packing (spec §11). The Appendix A vector is the primary check:
//! it is what distinguishes a conforming `>=` while-condition from `>`.

use arkavo_gguf_tdf::{GgufHeader, HeaderTensor, plan_segments};
use opentdf::GgufSegmentKind;

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

#[test]
fn appendix_a_packing_vector() {
    // ALIGN=32, headerBytes=64, maxSegment=128, a 256 B tensor then a 32 B one.
    let h = header(
        32,
        64,
        vec![
            tensor("token_embd.weight", 0, 256),
            tensor("blk.0.attn_norm.weight", 256, 32),
        ],
    );
    let plan = plan_segments(&h, 352, 128).unwrap();

    let got: Vec<_> = plan
        .iter()
        .map(|s| (s.id, s.kind, s.plain(), s.start, s.end, s.entry()))
        .collect();
    assert_eq!(
        got,
        vec![
            (0, GgufSegmentKind::Header, 64, 0, 64, "header".to_string()),
            (1, GgufSegmentKind::Tensor, 128, 64, 192, "s/1".to_string()),
            (2, GgufSegmentKind::Tensor, 128, 192, 320, "s/2".to_string()),
            (3, GgufSegmentKind::Tensor, 32, 320, 352, "s/3".to_string()),
        ],
        "a writer implementing `>` instead of `>=` would not emit s/2"
    );

    assert_eq!(plan.iter().map(|s| s.plain()).sum::<u64>(), 352);
}

#[test]
fn partitions_contiguously_and_never_exceeds_max_segment() {
    const MIB: u64 = 1024 * 1024;
    let h = header(
        32,
        4096,
        vec![tensor("a", 0, 16 * MIB), tensor("b", 16 * MIB, 4096)],
    );
    let virtual_size = 4096 + 16 * MIB + 4096;
    let plan = plan_segments(&h, virtual_size, 4 * MIB).unwrap();

    assert_eq!(plan[0].kind, GgufSegmentKind::Header);
    let mut cursor = 0u64;
    for s in &plan {
        assert_eq!(s.start, cursor, "segments must partition [0, virtualSize)");
        cursor = s.end;
        if s.id > 0 {
            assert!(
                s.plain() <= 4 * MIB,
                "segment {} is {} bytes, past the cap",
                s.id,
                s.plain()
            );
        }
    }
    assert_eq!(cursor, virtual_size);

    // A 16 MiB tensor at a 4 MiB cap becomes exactly four members.
    assert_eq!(plan.iter().filter(|s| (1..=4).contains(&s.id)).count(), 4);
    assert!(
        plan.iter()
            .all(|s| s.id == 0 || s.kind != GgufSegmentKind::Header)
    );
}

#[test]
fn ids_are_dense_and_match_their_entry_names() {
    const MIB: u64 = 1024 * 1024;
    let h = header(32, 4096, vec![tensor("a", 0, 10 * MIB)]);
    let plan = plan_segments(&h, 4096 + 10 * MIB, 4 * MIB).unwrap();

    for (i, s) in plan.iter().enumerate() {
        assert_eq!(s.id, i as u64, "ids must equal the array index");
    }
    assert_eq!(plan[0].entry(), "header");
    assert_eq!(plan[1].entry(), "s/1");
    assert_eq!(
        plan.last().unwrap().entry(),
        format!("s/{}", plan.len() - 1)
    );
}

#[test]
fn window_spanning_two_tensors_is_a_pack() {
    let h = header(32, 64, vec![tensor("a", 0, 64), tensor("b", 64, 64)]);
    let plan = plan_segments(&h, 192, 128).unwrap();

    assert_eq!(plan.len(), 2);
    assert_eq!(plan[1].kind, GgufSegmentKind::Pack);
    assert_eq!(plan[1].plain(), 128);
}

#[test]
fn padding_only_window_is_a_pack() {
    // One 32 B tensor then 96 B of trailing padding, with a 32 B cap: the
    // windows past the tensor touch no tensor at all.
    let h = header(32, 64, vec![tensor("a", 0, 32)]);
    let plan = plan_segments(&h, 192, 32).unwrap();

    assert_eq!(plan.iter().map(|s| s.plain()).sum::<u64>(), 192);
    let trailing: Vec<_> = plan.iter().filter(|s| s.start >= 96).collect();
    assert!(!trailing.is_empty());
    assert!(
        trailing.iter().all(|s| s.kind == GgufSegmentKind::Pack),
        "windows holding only padding are packs"
    );
}

#[test]
fn header_only_file_has_no_weight_segments() {
    let h = header(32, 128, vec![]);
    let plan = plan_segments(&h, 128, 128).unwrap();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].kind, GgufSegmentKind::Header);
    assert_eq!(plan[0].plain(), 128);
}

#[test]
fn header_may_exceed_max_segment() {
    // A large tokenizer pushes headerBytes past the cap; that is allowed for
    // segment 0 and only for segment 0.
    let h = header(32, 8192, vec![tensor("a", 0, 128)]);
    let plan = plan_segments(&h, 8192 + 128, 128).unwrap();
    assert_eq!(plan[0].plain(), 8192);
    assert!(plan[1..].iter().all(|s| s.plain() <= 128));
}

#[test]
fn rejects_overlapping_tensors() {
    let h = header(32, 64, vec![tensor("a", 0, 64), tensor("b", 32, 64)]);
    assert_eq!(
        plan_segments(&h, 256, 128).unwrap_err().code(),
        "GGUFTDF_OVERLAP"
    );
}

#[test]
fn rejects_misaligned_tensor_offset() {
    let h = header(32, 64, vec![tensor("a", 8, 64)]);
    assert_eq!(
        plan_segments(&h, 256, 128).unwrap_err().code(),
        "GGUFTDF_BAD_TENSOR"
    );
}

#[test]
fn rejects_max_segment_that_is_not_a_multiple_of_alignment() {
    let h = header(32, 64, vec![tensor("a", 0, 64)]);
    assert_eq!(
        plan_segments(&h, 256, 100).unwrap_err().code(),
        "GGUFTDF_BAD_MAX_SEGMENT"
    );
    // Also rejected below the alignment.
    assert_eq!(
        plan_segments(&h, 256, 16).unwrap_err().code(),
        "GGUFTDF_BAD_MAX_SEGMENT"
    );
}

#[test]
fn rejects_a_tensor_running_past_the_end_of_the_file() {
    let h = header(32, 64, vec![tensor("a", 0, 64)]);
    assert_eq!(
        plan_segments(&h, 100, 128).unwrap_err().code(),
        "GGUFTDF_BAD_TENSOR"
    );
}

#[test]
fn rejects_a_header_that_is_zero_or_misaligned_or_past_the_file() {
    let zero = header(32, 0, vec![]);
    assert_eq!(
        plan_segments(&zero, 128, 128).unwrap_err().code(),
        "GGUFTDF_BAD_HEADER"
    );

    let misaligned = header(32, 40, vec![]);
    assert_eq!(
        plan_segments(&misaligned, 128, 128).unwrap_err().code(),
        "GGUFTDF_BAD_HEADER"
    );

    let past_end = header(32, 256, vec![]);
    assert_eq!(
        plan_segments(&past_end, 128, 128).unwrap_err().code(),
        "GGUFTDF_BAD_HEADER"
    );
}

// --- Tensor-aligned boundaries (spec §11.3, draft-01) -------------------

/// Every tensor no larger than the cap must be covered by exactly one segment.
fn assert_small_tensors_unsplit(
    plan: &[arkavo_gguf_tdf::PlannedSegment],
    h: &GgufHeader,
    max_segment: u64,
) {
    for t in &h.tensors {
        if t.size > max_segment {
            continue;
        }
        let off = h.data_offset + t.gguf_offset;
        let end = off + t.size;
        let covering: Vec<_> = plan
            .iter()
            .filter(|s| s.start < end && off < s.end)
            .collect();
        assert_eq!(
            covering.len(),
            1,
            "tensor {} [{off}, {end}) is split across {:?}",
            t.name,
            covering
                .iter()
                .map(|s| (s.id, s.start, s.end))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn small_tensor_that_would_straddle_a_window_is_not_split() {
    // 96 B + 96 B with a 128 B cap. Fixed windows would cut the second tensor
    // at 192; tensor-aligned packing closes the pack at its start instead.
    let h = header(32, 64, vec![tensor("a", 0, 96), tensor("b", 96, 96)]);
    let plan = plan_segments(&h, 256, 128).unwrap();

    let got: Vec<_> = plan
        .iter()
        .map(|s| (s.id, s.kind, s.start, s.end))
        .collect();
    assert_eq!(
        got,
        vec![
            (0, GgufSegmentKind::Header, 0, 64),
            (1, GgufSegmentKind::Tensor, 64, 160),
            (2, GgufSegmentKind::Tensor, 160, 256),
        ]
    );
    assert_small_tensors_unsplit(&plan, &h, 128);
}

#[test]
fn large_tensor_after_an_open_pack_is_split_from_its_own_offset() {
    // 64 B tensor, then a 256 B tensor, 128 B cap. The pack closes at 128 and
    // the big tensor's windows start on its offset, not on the pack start.
    let h = header(32, 64, vec![tensor("a", 0, 64), tensor("b", 64, 256)]);
    let plan = plan_segments(&h, 384, 128).unwrap();

    let got: Vec<_> = plan.iter().map(|s| (s.kind, s.start, s.end)).collect();
    assert_eq!(
        got,
        vec![
            (GgufSegmentKind::Header, 0, 64),
            (GgufSegmentKind::Tensor, 64, 128),
            (GgufSegmentKind::Tensor, 128, 256),
            (GgufSegmentKind::Tensor, 256, 384),
        ]
    );
    assert_small_tensors_unsplit(&plan, &h, 128);
}

#[test]
fn remainder_of_a_split_tensor_packs_with_the_following_small_tensors() {
    // 160 B tensor then 32 B: one full window from the tensor offset, then
    // the 32 B remainder shares a pack with the next tensor.
    let h = header(32, 64, vec![tensor("a", 0, 160), tensor("b", 160, 32)]);
    let plan = plan_segments(&h, 256, 128).unwrap();

    let got: Vec<_> = plan.iter().map(|s| (s.kind, s.start, s.end)).collect();
    assert_eq!(
        got,
        vec![
            (GgufSegmentKind::Header, 0, 64),
            (GgufSegmentKind::Tensor, 64, 192),
            (GgufSegmentKind::Pack, 192, 256),
        ]
    );
    assert_small_tensors_unsplit(&plan, &h, 128);
}

#[test]
fn tensor_exactly_at_the_cap_closes_the_open_pack_and_gets_its_own_member() {
    let h = header(32, 64, vec![tensor("a", 0, 32), tensor("b", 32, 128)]);
    let plan = plan_segments(&h, 224, 128).unwrap();

    let got: Vec<_> = plan.iter().map(|s| (s.kind, s.start, s.end)).collect();
    assert_eq!(
        got,
        vec![
            (GgufSegmentKind::Header, 0, 64),
            (GgufSegmentKind::Tensor, 64, 96),
            (GgufSegmentKind::Tensor, 96, 224),
        ]
    );
    assert_small_tensors_unsplit(&plan, &h, 128);
}

#[test]
fn a_layer_of_mixed_tensors_never_splits_a_small_one() {
    // A transformer-ish layer: a few big projections and many small norms,
    // with a cap that is not a multiple of any of them.
    const KIB: u64 = 1024;
    let mut tensors = Vec::new();
    let mut off = 0;
    for layer in 0..4 {
        for (name, size) in [
            ("attn_q", 300 * KIB),
            ("attn_norm", 3 * KIB),
            ("attn_k", 700 * KIB),
            ("ffn_norm", 5 * KIB),
            ("ffn_up", 1500 * KIB),
        ] {
            tensors.push(tensor(&format!("blk.{layer}.{name}"), off, size));
            off += size;
            off = off.div_ceil(32) * 32;
        }
    }
    let virtual_size = 4096 + off;
    let h = header(32, 4096, tensors);
    let max = 512 * KIB;
    let plan = plan_segments(&h, virtual_size, max).unwrap();

    assert_small_tensors_unsplit(&plan, &h, max);
    let mut cursor = 0;
    for s in &plan {
        assert_eq!(s.start, cursor);
        cursor = s.end;
        assert!(s.id == 0 || s.plain() <= max);
        assert_eq!(s.start % 32, 0, "boundaries stay aligned");
    }
    assert_eq!(cursor, virtual_size);
}
