//! Hybrid index construction and open-time validation (spec §9).
//!
//! The index lets `read_at` map an offset to a zip member without first
//! decrypting the header. Validation runs on zip and JSON alone, so a
//! malformed archive is rejected before any KAS round-trip.

use crate::error::GgufTdfError;
use crate::gguf_header::{GgufHeader, MAX_TENSOR_NAME_BYTES};
use crate::pack::PlannedSegment;
use crate::{PROFILE, SEGMENT_OVERHEAD};
use opentdf::TdfMemberIndex;
use opentdf::{GgufIndex, GgufSegment, GgufSegmentKind, GgufTensor, IntegrityInformation};
use std::collections::HashSet;

/// Virtual start offset of every segment, for binary-search lookup.
#[derive(Debug, Clone)]
pub struct SegmentMap {
    /// `starts[i]` is the virtual offset where segment `i` begins;
    /// `starts[len]` is `virtualSize`.
    starts: Vec<u64>,
}

impl SegmentMap {
    /// Builds prefix sums over the index's segment sizes.
    pub fn new(index: &GgufIndex) -> Self {
        let mut starts = Vec::with_capacity(index.segments.len() + 1);
        let mut acc = 0u64;
        for seg in &index.segments {
            starts.push(acc);
            acc = acc.saturating_add(seg.plain);
        }
        starts.push(acc);
        Self { starts }
    }

    /// Index of the segment covering `offset`, or `None` past the end.
    pub fn covering(&self, offset: u64) -> Option<usize> {
        let last = *self.starts.last()?;
        if offset >= last {
            return None;
        }
        // `partition_point` gives the count of starts <= offset; the covering
        // segment is the one just before that boundary.
        Some(self.starts.partition_point(|s| *s <= offset) - 1)
    }

    /// Virtual start offset of segment `id`.
    pub fn start_of(&self, id: usize) -> u64 {
        self.starts[id]
    }

    /// Total virtual size described by the map.
    pub fn virtual_size(&self) -> u64 {
        *self.starts.last().unwrap_or(&0)
    }

    /// Number of segments.
    pub fn len(&self) -> usize {
        self.starts.len().saturating_sub(1)
    }

    /// Whether the map describes no segments.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Builds the plaintext `gguf` index from a header and its segment plan.
pub fn build_index(
    header: &GgufHeader,
    plan: &[PlannedSegment],
    virtual_size: u64,
    max_segment: u64,
) -> Result<GgufIndex, GgufTdfError> {
    let segments: Vec<GgufSegment> = plan
        .iter()
        .map(|s| GgufSegment {
            id: s.id,
            kind: s.kind,
            plain: s.plain(),
            entry: s.entry(),
        })
        .collect();

    let mut index = GgufIndex {
        profile: PROFILE.to_string(),
        alignment: header.alignment,
        header_bytes: header.data_offset,
        virtual_size,
        max_segment,
        tensors: Vec::with_capacity(header.tensors.len()),
        segments,
    };

    let map = SegmentMap::new(&index);
    for t in &header.tensors {
        let offset = header.data_offset + t.gguf_offset;
        // Half-open [start, end): the segment holding the tensor's first byte
        // through the segment holding its last. Those segments may also hold
        // other tensors and padding, so this is containment, not equality.
        let start = map.covering(offset).ok_or_else(|| {
            GgufTdfError::BadIndex(format!("tensor '{}' starts past the file", t.name))
        })? as u64;
        let last_byte = offset + t.size.saturating_sub(1);
        let end = map.covering(last_byte).ok_or_else(|| {
            GgufTdfError::BadIndex(format!("tensor '{}' ends past the file", t.name))
        })? as u64
            + 1;

        index.tensors.push(GgufTensor {
            name: t.name.clone(),
            offset,
            size: t.size,
            segments: [start, end],
        });
    }

    Ok(index)
}

/// Verifies spec §9.4 invariants 1–11 using only the zip and the manifest.
///
/// Returns the prefix-sum map so the reader does not recompute it.
pub fn validate_index(
    index: &GgufIndex,
    integrity: &IntegrityInformation,
    members: &TdfMemberIndex,
) -> Result<SegmentMap, GgufTdfError> {
    if index.profile != PROFILE {
        return Err(GgufTdfError::UnsupportedProfile(index.profile.clone()));
    }
    if index.alignment < 8 || !index.alignment.is_power_of_two() {
        return Err(GgufTdfError::BadAlign(index.alignment));
    }
    // Invariant 9.
    if !index.max_segment.is_multiple_of(index.alignment) || index.max_segment < index.alignment {
        return Err(GgufTdfError::BadMaxSegment(index.max_segment));
    }
    if index.header_bytes == 0 || !index.header_bytes.is_multiple_of(index.alignment) {
        return Err(GgufTdfError::BadHeader(format!(
            "headerBytes {} must be non-zero and a multiple of {}",
            index.header_bytes, index.alignment
        )));
    }
    if index.virtual_size < index.header_bytes {
        return Err(GgufTdfError::BadHeader(
            "virtualSize is smaller than headerBytes".to_string(),
        ));
    }

    // Invariant 4.
    if index.segments.len() != integrity.segments.len() {
        return Err(GgufTdfError::BadIndex(format!(
            "gguf.segments has {} entries but integrityInformation.segments has {}",
            index.segments.len(),
            integrity.segments.len()
        )));
    }
    if index.segments.is_empty() {
        return Err(GgufTdfError::BadIndex(
            "an archive must have at least the header segment".to_string(),
        ));
    }

    // Invariant 3.
    let head = &index.segments[0];
    if head.id != 0
        || head.kind != GgufSegmentKind::Header
        || head.entry != crate::HEADER_ENTRY
        || head.plain != index.header_bytes
    {
        return Err(GgufTdfError::BadIndex(
            "segment 0 must be the header, named `header`, of headerBytes length".to_string(),
        ));
    }

    validate_segments(index, integrity, members)?;

    let map = SegmentMap::new(index);
    // Invariants 1 and 2: the prefix sums land exactly on virtualSize, which
    // for a contiguous partition is the same statement.
    if map.virtual_size() != index.virtual_size {
        return Err(GgufTdfError::SizeMismatch);
    }

    validate_tensors(index, &map)?;

    Ok(map)
}

fn validate_segments(
    index: &GgufIndex,
    integrity: &IntegrityInformation,
    members: &TdfMemberIndex,
) -> Result<(), GgufTdfError> {
    for (i, seg) in index.segments.iter().enumerate() {
        if seg.id != i as u64 {
            return Err(GgufTdfError::BadIndex(format!(
                "segment at index {i} declares id {}",
                seg.id
            )));
        }
        if seg.entry != crate::entry_name(seg.id) {
            return Err(GgufTdfError::BadIndex(format!(
                "segment {} has entry {:?}, expected {:?}",
                seg.id,
                seg.entry,
                crate::entry_name(seg.id)
            )));
        }
        if i > 0 && seg.kind == GgufSegmentKind::Header {
            return Err(GgufTdfError::BadIndex(
                "only segment 0 may be the header".to_string(),
            ));
        }
        // Invariant 10. Invariant 11 is the absence of this check for id 0.
        if i > 0 && seg.plain > index.max_segment {
            return Err(GgufTdfError::BadIndex(format!(
                "segment {} is {} bytes, past maxSegment {}",
                seg.id, seg.plain, index.max_segment
            )));
        }
        if seg.plain == 0 {
            return Err(GgufTdfError::BadIndex(format!(
                "segment {} is empty",
                seg.id
            )));
        }

        let row = &integrity.segments[i];
        let segment_size = row.segment_size.ok_or_else(|| {
            GgufTdfError::BadIndex(format!("integrity row {i} omits segmentSize"))
        })?;
        let encrypted_size = row.encrypted_segment_size.ok_or_else(|| {
            GgufTdfError::BadIndex(format!("integrity row {i} omits encryptedSegmentSize"))
        })?;

        if segment_size != seg.plain {
            return Err(GgufTdfError::BadIndex(format!(
                "segment {} plain {} disagrees with segmentSize {segment_size}",
                seg.id, seg.plain
            )));
        }
        // Invariant 6.
        if encrypted_size != segment_size + SEGMENT_OVERHEAD {
            return Err(GgufTdfError::BadIndex(format!(
                "segment {} encryptedSegmentSize {encrypted_size} is not segmentSize + {SEGMENT_OVERHEAD}",
                seg.id
            )));
        }

        // Invariant 5.
        let location = members
            .get(&seg.entry)
            .ok_or_else(|| GgufTdfError::BadIndex(format!("zip has no member {:?}", seg.entry)))?;
        if location.size != encrypted_size {
            return Err(GgufTdfError::BadIndex(format!(
                "member {:?} is {} bytes, expected {encrypted_size}",
                seg.entry, location.size
            )));
        }
    }
    Ok(())
}

fn validate_tensors(index: &GgufIndex, map: &SegmentMap) -> Result<(), GgufTdfError> {
    let mut seen = HashSet::with_capacity(index.tensors.len());
    let mut previous_end = index.header_bytes;

    for t in &index.tensors {
        // Invariant 7.
        if !seen.insert(t.name.as_str()) {
            return Err(GgufTdfError::BadIndex(format!(
                "duplicate tensor name {:?}",
                t.name
            )));
        }
        if t.name.len() > MAX_TENSOR_NAME_BYTES {
            return Err(GgufTdfError::BadIndex(format!(
                "tensor name {:?} is {} UTF-8 bytes; ggml rejects 64 or more",
                t.name,
                t.name.len()
            )));
        }
        if t.offset < index.header_bytes {
            return Err(GgufTdfError::BadIndex(format!(
                "tensor {:?} starts inside the header",
                t.name
            )));
        }
        if t.offset < previous_end {
            return Err(GgufTdfError::BadIndex(format!(
                "tensor {:?} is not strictly after the previous tensor",
                t.name
            )));
        }
        let end = t
            .offset
            .checked_add(t.size)
            .ok_or_else(|| GgufTdfError::BadIndex("tensor extent overflow".to_string()))?;
        if end > index.virtual_size {
            return Err(GgufTdfError::BadIndex(format!(
                "tensor {:?} ends past virtualSize",
                t.name
            )));
        }
        previous_end = end;

        // Invariant 8: containment, not equality — the bounding segments may
        // also hold other tensors and alignment padding.
        let [start, seg_end] = t.segments;
        if seg_end <= start || start < 1 || seg_end > index.segments.len() as u64 {
            return Err(GgufTdfError::BadIndex(format!(
                "tensor {:?} has segment range [{start}, {seg_end})",
                t.name
            )));
        }
        let expected_start = map.covering(t.offset).ok_or_else(|| {
            GgufTdfError::BadIndex(format!("tensor {:?} starts past the file", t.name))
        })? as u64;
        let expected_end = map
            .covering(t.offset + t.size.saturating_sub(1))
            .ok_or_else(|| {
                GgufTdfError::BadIndex(format!("tensor {:?} ends past the file", t.name))
            })? as u64
            + 1;
        if start != expected_start || seg_end != expected_end {
            return Err(GgufTdfError::BadIndex(format!(
                "tensor {:?} declares segments [{start}, {seg_end}) but occupies [{expected_start}, {expected_end})",
                t.name
            )));
        }
    }
    Ok(())
}
