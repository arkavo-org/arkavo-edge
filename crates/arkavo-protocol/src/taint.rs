//! Taint labels and monotonic propagation (SEQ-001, SEQ-002).
//!
//! A [`TaintSet`] answers one question for a downstream gate: how sensitive is
//! this buffer, and where did the sensitivity come from. Every operation here
//! is monotonic — a union may raise sensitivity or add categories, never the
//! reverse — because a transformation the attacker chooses (encode, extract,
//! summarize) must not be able to wash a label off the data it carries.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::data_classification::{ClassifiedDatum, DataCategory, SensitivityLevel};

/// Provenance chains are attacker-influenced: a loop of transformations would
/// otherwise grow one without bound. Hops past this are counted, not kept.
pub const MAX_PROVENANCE_HOPS: usize = 64;

/// Distinct sources in one set are bounded the same way. Excess labels fold
/// into [`AGGREGATE_SOURCE_ID`] rather than being dropped, so the set's
/// sensitivity and categories survive the bound.
pub const MAX_LABELS: usize = 128;

/// Source id of the label that absorbs sources past [`MAX_LABELS`].
pub const AGGREGATE_SOURCE_ID: &str = "arkavo:aggregate";

/// Where data entered the agent's context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    UserInput,
    ToolResult,
    FileRead,
    A2aReceive,
    ModelOutput,
    /// Origin could not be determined. Treated conservatively.
    Unknown,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::UserInput => "user",
            SourceKind::ToolResult => "tool",
            SourceKind::FileRead => "file",
            SourceKind::A2aReceive => "a2a",
            SourceKind::ModelOutput => "model",
            SourceKind::Unknown => "unknown",
        }
    }
}

/// An ingestion point, plus whatever the caller can assert about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaintSource {
    pub kind: SourceKind,
    /// Identifier within the kind: a tool name, a path, a peer DID.
    pub id: String,
    /// Sensitivity the caller can vouch for. `None` means unknown, which the
    /// tracker resolves conservatively rather than as public.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared: Option<SensitivityLevel>,
}

impl TaintSource {
    pub fn new(kind: SourceKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
            declared: None,
        }
    }

    /// Assert a known sensitivity for this source. Declaring `Public` is the
    /// only way to opt a source out of the conservative floor.
    pub fn declared(mut self, level: SensitivityLevel) -> Self {
        self.declared = Some(level);
        self
    }

    /// Stable label key: distinct sources must not collide, or a union would
    /// silently merge two provenance chains.
    pub fn source_id(&self) -> String {
        format!("{}:{}", self.kind.as_str(), self.id)
    }
}

/// What was done to the data between one label state and the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transformation {
    Encode,
    Decode,
    Extract,
    Summarize,
    Chunk,
    Merge,
    Inference,
    Other,
}

impl Transformation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Transformation::Encode => "encode",
            Transformation::Decode => "decode",
            Transformation::Extract => "extract",
            Transformation::Summarize => "summarize",
            Transformation::Chunk => "chunk",
            Transformation::Merge => "merge",
            Transformation::Inference => "inference",
            Transformation::Other => "other",
        }
    }
}

/// One step in a provenance chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceHop {
    pub transformation: Transformation,
    /// What performed it: a tool name, a codec, a model id.
    pub detail: String,
}

impl ProvenanceHop {
    pub fn new(transformation: Transformation, detail: impl Into<String>) -> Self {
        Self {
            transformation,
            detail: detail.into(),
        }
    }
}

/// Everything one source contributed to a buffer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaintLabel {
    pub source_id: String,
    pub categories: BTreeSet<DataCategory>,
    pub sensitivity: SensitivityLevel,
    pub hops: Vec<ProvenanceHop>,
    /// Hops dropped at [`MAX_PROVENANCE_HOPS`]. Non-zero means the chain is
    /// incomplete for forensics, which an auditor has to be able to see.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub truncated_hops: u32,
}

// serde's `skip_serializing_if` takes `fn(&T) -> bool`, so this cannot accept
// `u32` by value however small the type is.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl TaintLabel {
    pub fn new(
        source_id: impl Into<String>,
        categories: impl IntoIterator<Item = DataCategory>,
        sensitivity: SensitivityLevel,
    ) -> Self {
        Self {
            source_id: source_id.into(),
            categories: categories.into_iter().collect(),
            sensitivity,
            hops: Vec::new(),
            truncated_hops: 0,
        }
    }

    /// Build a label from what a classifier found, never below `floor`.
    /// An empty finding list is not evidence of safety, so the floor still
    /// applies (SEQ-001: conservative classification on ambiguity).
    pub fn from_classifications(
        source_id: impl Into<String>,
        found: &[ClassifiedDatum],
        floor: SensitivityLevel,
    ) -> Self {
        let mut categories: BTreeSet<DataCategory> = BTreeSet::new();
        let mut sensitivity = floor;
        for datum in found {
            categories.insert(datum.category());
            sensitivity = sensitivity.max(datum.sensitivity());
        }
        Self {
            source_id: source_id.into(),
            categories,
            sensitivity,
            hops: Vec::new(),
            truncated_hops: 0,
        }
    }

    pub fn with_hop(mut self, hop: ProvenanceHop) -> Self {
        self.push_hop(hop);
        self
    }

    pub fn push_hop(&mut self, hop: ProvenanceHop) {
        if self.hops.len() >= MAX_PROVENANCE_HOPS {
            self.truncated_hops = self.truncated_hops.saturating_add(1);
            return;
        }
        self.hops.push(hop);
    }

    /// Merge another label for the same source. Monotonic in both directions
    /// it can move: sensitivity rises, categories accumulate.
    pub fn absorb(&mut self, other: &TaintLabel) {
        self.sensitivity = self.sensitivity.max(other.sensitivity);
        self.categories.extend(other.categories.iter().copied());
        for hop in &other.hops {
            if self.hops.contains(hop) {
                continue;
            }
            self.push_hop(hop.clone());
        }
        self.truncated_hops = self.truncated_hops.saturating_add(other.truncated_hops);
    }
}

/// The taint carried by one buffer, keyed by source so a union can coalesce
/// two mentions of the same origin without losing either provenance chain.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaintSet {
    labels: BTreeMap<String, TaintLabel>,
}

impl TaintSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_label(label: TaintLabel) -> Self {
        let mut set = Self::new();
        set.insert(label);
        set
    }

    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    pub fn len(&self) -> usize {
        self.labels.len()
    }

    pub fn labels(&self) -> impl Iterator<Item = &TaintLabel> {
        self.labels.values()
    }

    pub fn source_ids(&self) -> impl Iterator<Item = &str> {
        self.labels.keys().map(String::as_str)
    }

    pub fn label_for(&self, source_id: &str) -> Option<&TaintLabel> {
        self.labels.get(source_id)
    }

    /// Add a label, coalescing with any label already held for that source.
    /// Past [`MAX_LABELS`] the new label folds into the aggregate label, which
    /// keeps the set's sensitivity and categories correct under the bound.
    pub fn insert(&mut self, label: TaintLabel) {
        if let Some(existing) = self.labels.get_mut(&label.source_id) {
            existing.absorb(&label);
            return;
        }
        if self.labels.len() >= MAX_LABELS && label.source_id != AGGREGATE_SOURCE_ID {
            let mut folded = label;
            folded.source_id = AGGREGATE_SOURCE_ID.to_string();
            self.insert(folded);
            return;
        }
        self.labels.insert(label.source_id.clone(), label);
    }

    /// Monotonic union: max sensitivity, union of categories, provenance of
    /// both sides retained per source.
    pub fn merge(&mut self, other: &TaintSet) {
        for label in other.labels.values() {
            self.insert(label.clone());
        }
    }

    #[must_use]
    pub fn union(mut self, other: &TaintSet) -> Self {
        self.merge(other);
        self
    }

    /// Highest sensitivity anything in this buffer carries. An empty set is
    /// `Public` — the absence of a label, not a claim about unlabelled data;
    /// callers that ingest through a tracker never see an empty set.
    pub fn sensitivity(&self) -> SensitivityLevel {
        self.labels
            .values()
            .map(|l| l.sensitivity)
            .max()
            .unwrap_or(SensitivityLevel::Public)
    }

    pub fn categories(&self) -> BTreeSet<DataCategory> {
        self.labels
            .values()
            .flat_map(|l| l.categories.iter().copied())
            .collect()
    }

    pub fn contains_category(&self, category: DataCategory) -> bool {
        self.labels
            .values()
            .any(|l| l.categories.contains(&category))
    }

    /// Result of transforming this buffer. Every label keeps its origin and
    /// gains a hop, which is what makes encoding a traceable step rather than
    /// an escape (SEQ-002: encoding changes do not strip taint).
    #[must_use]
    pub fn transformed(&self, transformation: Transformation, detail: &str) -> TaintSet {
        let hop = ProvenanceHop::new(transformation, detail);
        let mut out = self.clone();
        for label in out.labels.values_mut() {
            label.push_hop(hop.clone());
        }
        out
    }

    /// Raise every label to at least `ceiling`, adding one if the set is empty.
    /// Used for a serving model's classification ceiling, which output inherits
    /// whether or not the input carried anything.
    #[must_use]
    pub fn raised_to(&self, source_id: &str, ceiling: SensitivityLevel) -> TaintSet {
        let mut out = self.clone();
        if out.labels.is_empty() {
            out.insert(TaintLabel::new(source_id, [], ceiling));
            return out;
        }
        for label in out.labels.values_mut() {
            label.sensitivity = label.sensitivity.max(ceiling);
        }
        out
    }
}

impl FromIterator<TaintLabel> for TaintSet {
    fn from_iter<I: IntoIterator<Item = TaintLabel>>(iter: I) -> Self {
        let mut set = Self::new();
        for label in iter {
            set.insert(label);
        }
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_classification::DatumType;

    fn label(source: &str, category: DataCategory, level: SensitivityLevel) -> TaintLabel {
        TaintLabel::new(source, [category], level)
    }

    #[test]
    fn empty_set_is_public() {
        let set = TaintSet::new();
        assert!(set.is_empty());
        assert_eq!(set.sensitivity(), SensitivityLevel::Public);
    }

    #[test]
    fn union_takes_the_highest_sensitivity() {
        let low = TaintSet::from_label(label(
            "file:notes",
            DataCategory::Internal,
            SensitivityLevel::Internal,
        ));
        let high = TaintSet::from_label(label(
            "tool:vault",
            DataCategory::Credentials,
            SensitivityLevel::Restricted,
        ));

        let merged = low.union(&high);

        assert_eq!(merged.sensitivity(), SensitivityLevel::Restricted);
    }

    #[test]
    fn union_is_commutative_in_sensitivity_and_categories() {
        let a = TaintSet::from_label(label(
            "file:a",
            DataCategory::Pii,
            SensitivityLevel::Internal,
        ));
        let b = TaintSet::from_label(label(
            "file:b",
            DataCategory::Financial,
            SensitivityLevel::Confidential,
        ));

        let ab = a.clone().union(&b);
        let ba = b.union(&a);

        assert_eq!(ab.sensitivity(), ba.sensitivity());
        assert_eq!(ab.categories(), ba.categories());
    }

    #[test]
    fn union_never_lowers_an_existing_label() {
        let mut set = TaintSet::from_label(label(
            "tool:vault",
            DataCategory::Credentials,
            SensitivityLevel::Restricted,
        ));
        let downgrade = TaintSet::from_label(label(
            "tool:vault",
            DataCategory::Public,
            SensitivityLevel::Public,
        ));

        set.merge(&downgrade);

        assert_eq!(set.sensitivity(), SensitivityLevel::Restricted);
        assert!(set.contains_category(DataCategory::Credentials));
    }

    #[test]
    fn union_accumulates_categories_for_one_source() {
        let mut set = TaintSet::from_label(label(
            "tool:crm",
            DataCategory::Pii,
            SensitivityLevel::Internal,
        ));
        set.merge(&TaintSet::from_label(label(
            "tool:crm",
            DataCategory::Financial,
            SensitivityLevel::Internal,
        )));

        assert_eq!(set.len(), 1);
        assert!(set.contains_category(DataCategory::Pii));
        assert!(set.contains_category(DataCategory::Financial));
    }

    #[test]
    fn transformation_records_a_hop_without_changing_sensitivity() {
        let set = TaintSet::from_label(label(
            "file:secrets",
            DataCategory::Credentials,
            SensitivityLevel::Restricted,
        ));

        let encoded = set.transformed(Transformation::Encode, "base64");

        assert_eq!(encoded.sensitivity(), SensitivityLevel::Restricted);
        let label = encoded.label_for("file:secrets").expect("label survives");
        assert_eq!(label.hops.len(), 1);
        assert_eq!(label.hops[0].transformation, Transformation::Encode);
        assert_eq!(label.hops[0].detail, "base64");
    }

    #[test]
    fn transformation_chains_accumulate_in_order() {
        let set = TaintSet::from_label(label(
            "file:secrets",
            DataCategory::Internal,
            SensitivityLevel::Internal,
        ))
        .transformed(Transformation::Encode, "base64")
        .transformed(Transformation::Summarize, "model");

        let hops = &set.label_for("file:secrets").expect("label").hops;
        assert_eq!(hops[0].transformation, Transformation::Encode);
        assert_eq!(hops[1].transformation, Transformation::Summarize);
    }

    #[test]
    fn provenance_chain_is_bounded_and_reports_truncation() {
        let mut set = TaintSet::from_label(label(
            "file:loop",
            DataCategory::Internal,
            SensitivityLevel::Internal,
        ));
        for i in 0..(MAX_PROVENANCE_HOPS + 10) {
            set = set.transformed(Transformation::Encode, &format!("codec-{i}"));
        }

        let label = set.label_for("file:loop").expect("label");
        assert_eq!(label.hops.len(), MAX_PROVENANCE_HOPS);
        assert_eq!(label.truncated_hops, 10);
        assert_eq!(set.sensitivity(), SensitivityLevel::Internal);
    }

    #[test]
    fn label_count_is_bounded_without_losing_sensitivity() {
        let mut set = TaintSet::new();
        for i in 0..(MAX_LABELS + 5) {
            set.insert(label(
                &format!("tool:t{i}"),
                DataCategory::Internal,
                SensitivityLevel::Internal,
            ));
        }
        set.insert(label(
            "tool:overflow",
            DataCategory::Credentials,
            SensitivityLevel::Restricted,
        ));

        assert!(set.len() <= MAX_LABELS + 1);
        assert_eq!(set.sensitivity(), SensitivityLevel::Restricted);
        assert!(set.contains_category(DataCategory::Credentials));
        assert!(set.label_for(AGGREGATE_SOURCE_ID).is_some());
    }

    #[test]
    fn raising_to_a_ceiling_labels_an_otherwise_clean_set() {
        let set = TaintSet::new().raised_to("model:local", SensitivityLevel::Confidential);

        assert_eq!(set.sensitivity(), SensitivityLevel::Confidential);
        assert!(set.label_for("model:local").is_some());
    }

    #[test]
    fn raising_to_a_lower_ceiling_leaves_labels_alone() {
        let set = TaintSet::from_label(label(
            "tool:vault",
            DataCategory::Credentials,
            SensitivityLevel::Restricted,
        ))
        .raised_to("model:local", SensitivityLevel::Internal);

        assert_eq!(set.sensitivity(), SensitivityLevel::Restricted);
    }

    #[test]
    fn classifications_build_a_label_at_the_highest_finding() {
        let found = vec![
            ClassifiedDatum {
                datum_type: DatumType::Email,
                position: (0, 5),
                matched_text: "a@b.c".into(),
            },
            ClassifiedDatum {
                datum_type: DatumType::ApiKey,
                position: (6, 15),
                matched_text: "sk-abc123".into(),
            },
        ];

        let label = TaintLabel::from_classifications("tool:read", &found, SensitivityLevel::Public);

        assert_eq!(label.sensitivity, SensitivityLevel::Restricted);
        assert!(label.categories.contains(&DataCategory::Pii));
        assert!(label.categories.contains(&DataCategory::Credentials));
    }

    #[test]
    fn a_clean_scan_still_honors_the_conservative_floor() {
        let label = TaintLabel::from_classifications("tool:read", &[], SensitivityLevel::Internal);

        assert_eq!(label.sensitivity, SensitivityLevel::Internal);
        assert!(label.categories.is_empty());
    }

    #[test]
    fn source_ids_distinguish_kinds_with_the_same_name() {
        let tool = TaintSource::new(SourceKind::ToolResult, "report");
        let file = TaintSource::new(SourceKind::FileRead, "report");

        assert_ne!(tool.source_id(), file.source_id());
    }

    #[test]
    fn set_round_trips_through_serde() {
        let set = TaintSet::from_label(
            label(
                "a2a:did:key:z6Mk",
                DataCategory::Pii,
                SensitivityLevel::Confidential,
            )
            .with_hop(ProvenanceHop::new(Transformation::Summarize, "gemma")),
        );

        let json = serde_json::to_string(&set).expect("serialize");
        let back: TaintSet = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(set, back);
    }
}
