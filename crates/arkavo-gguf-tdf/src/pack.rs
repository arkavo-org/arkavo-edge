//! Segment planner (spec §11).
//!
//! Turns a parsed GGUF header into the virtual ranges that become zip
//! members. The plan is computed from the header alone, so a writer never
//! loads weights.

use crate::error::GgufTdfError;
use crate::gguf_header::GgufHeader;
use opentdf::GgufSegmentKind;

/// One planned segment as a half-open virtual range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSegment {
    /// Segment id; equals the index in the plan. Header is 0.
    pub id: u64,
    pub kind: GgufSegmentKind,
    /// Inclusive virtual start offset.
    pub start: u64,
    /// Exclusive virtual end offset.
    pub end: u64,
}

impl PlannedSegment {
    /// Plaintext length of this segment.
    pub fn plain(&self) -> u64 {
        self.end - self.start
    }

    /// Zip member name for this segment.
    pub fn entry(&self) -> String {
        crate::entry_name(self.id)
    }
}

/// Virtual `[start, end)` ranges of every tensor, in GGUF order.
struct TensorRanges(Vec<(u64, u64)>);

impl TensorRanges {
    /// How many tensors a window overlaps. Ranges are sorted and disjoint, so
    /// the overlapping run is a contiguous slice found by two binary searches.
    fn count_intersecting(&self, start: u64, end: u64) -> usize {
        let lo = self.0.partition_point(|(_, t_end)| *t_end <= start);
        let hi = self.0.partition_point(|(t_start, _)| *t_start < end);
        hi.saturating_sub(lo)
    }

    /// Classifies a window: exactly one tensor is `Tensor`; zero (padding
    /// only) or two or more is `Pack`.
    fn kind_for(&self, start: u64, end: u64) -> GgufSegmentKind {
        if self.count_intersecting(start, end) == 1 {
            GgufSegmentKind::Tensor
        } else {
            GgufSegmentKind::Pack
        }
    }
}

/// Builds the segment plan for a source GGUF (spec §11.3).
///
/// The while-conditions are `>=`, not `>`: a remainder of exactly
/// `max_segment` must still emit its own member.
pub fn plan_segments(
    header: &GgufHeader,
    virtual_size: u64,
    max_segment: u64,
) -> Result<Vec<PlannedSegment>, GgufTdfError> {
    let align = header.alignment;
    let header_bytes = header.data_offset;

    validate_inputs(header, virtual_size, max_segment)?;

    let ranges = tensor_ranges(header, virtual_size, align)?;

    let mut plan = Vec::new();
    plan.push(PlannedSegment {
        id: 0,
        kind: GgufSegmentKind::Header,
        start: 0,
        end: header_bytes,
    });

    let mut pack_start = header_bytes;
    let mut cursor = header_bytes;
    let mut next_id = 1u64;

    for &(t_off, t_end) in &ranges.0 {
        if t_off < cursor {
            return Err(GgufTdfError::Overlap);
        }
        let mut remaining_off = t_off;

        // Close an open pack that already holds a full cap of earlier
        // tensors and padding, before taking any bytes from this tensor.
        while remaining_off - pack_start >= max_segment {
            let end = pack_start + max_segment;
            push(&mut plan, &mut next_id, &ranges, pack_start, end);
            pack_start = end;
        }

        // Split this tensor while a full window still fits.
        while t_end - pack_start >= max_segment && t_end > remaining_off {
            let end = pack_start + max_segment;
            push(&mut plan, &mut next_id, &ranges, pack_start, end);
            pack_start = end;
            remaining_off = pack_start;
        }

        cursor = t_end;
    }

    // Everything left, including trailing padding. Split it too: the profile
    // caps every non-header segment at max_segment (§9.4 invariant 10), and a
    // file may end with more than one window of padding.
    while virtual_size - pack_start >= max_segment {
        let end = pack_start + max_segment;
        push(&mut plan, &mut next_id, &ranges, pack_start, end);
        pack_start = end;
    }
    if virtual_size > pack_start {
        push(&mut plan, &mut next_id, &ranges, pack_start, virtual_size);
    }

    debug_assert_eq!(
        plan.iter().map(PlannedSegment::plain).sum::<u64>(),
        virtual_size,
        "segments must partition the virtual file"
    );

    Ok(plan)
}

fn push(
    plan: &mut Vec<PlannedSegment>,
    next_id: &mut u64,
    ranges: &TensorRanges,
    start: u64,
    end: u64,
) {
    plan.push(PlannedSegment {
        id: *next_id,
        kind: ranges.kind_for(start, end),
        start,
        end,
    });
    *next_id += 1;
}

/// Spec §11.2 pre-packing checks that depend only on scalars.
fn validate_inputs(
    header: &GgufHeader,
    virtual_size: u64,
    max_segment: u64,
) -> Result<(), GgufTdfError> {
    let align = header.alignment;
    if align < 8 || !align.is_power_of_two() {
        return Err(GgufTdfError::BadAlign(align));
    }
    if max_segment < align || !max_segment.is_multiple_of(align) {
        return Err(GgufTdfError::BadMaxSegment(max_segment));
    }

    let header_bytes = header.data_offset;
    if header_bytes == 0 || !header_bytes.is_multiple_of(align) {
        return Err(GgufTdfError::BadHeader(format!(
            "headerBytes {header_bytes} must be non-zero and a multiple of {align}"
        )));
    }
    if header_bytes > virtual_size {
        return Err(GgufTdfError::BadHeader(format!(
            "headerBytes {header_bytes} exceeds virtualSize {virtual_size}"
        )));
    }
    Ok(())
}

/// Virtual ranges of every tensor, with the §11.2 geometry checks.
fn tensor_ranges(
    header: &GgufHeader,
    virtual_size: u64,
    align: u64,
) -> Result<TensorRanges, GgufTdfError> {
    let header_bytes = header.data_offset;
    let mut ranges = Vec::with_capacity(header.tensors.len());

    for t in &header.tensors {
        if !t.gguf_offset.is_multiple_of(align) {
            return Err(GgufTdfError::BadTensor(format!(
                "tensor '{}' offset {} is not a multiple of alignment {align}",
                t.name, t.gguf_offset
            )));
        }
        let start = header_bytes
            .checked_add(t.gguf_offset)
            .ok_or_else(|| GgufTdfError::BadTensor("tensor offset overflow".to_string()))?;
        let end = start
            .checked_add(t.size)
            .ok_or_else(|| GgufTdfError::BadTensor("tensor extent overflow".to_string()))?;
        if end > virtual_size {
            return Err(GgufTdfError::BadTensor(format!(
                "tensor '{}' ends at {end}, past virtualSize {virtual_size}",
                t.name
            )));
        }
        ranges.push((start, end));
    }

    // The packer walks tensors in file order and relies on them being sorted
    // and disjoint; the ordering check here makes the later binary searches
    // sound, and overlap is reported per the spec while packing.
    if ranges.windows(2).any(|w| w[1].0 < w[0].0) {
        return Err(GgufTdfError::BadTensor(
            "tensor offsets are not strictly increasing in GGUF order".to_string(),
        ));
    }

    Ok(TensorRanges(ranges))
}
