//! The semantic section of the index component.
//!
//! Vectors of protected text are invertible, so unlike the keyed tiers this
//! section is safe only because the component is sealed. It stores no text and
//! no offsets: a label, a family id and an int8 vector per unit.

use std::collections::{BTreeMap, BTreeSet};

use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::embed::{Embedder, EmbeddingPooling, QuantizedVector, embed_units};

pub const SEMANTIC_FORMAT_VERSION: &str = "1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedderRecord {
    /// `<hf-repo>/<file>` a node fetches the model from.
    pub source: String,
    pub sha256: String,
    pub pooling: EmbeddingPooling,
}

/// "category:sensitivity" in the taxonomy's serde names, e.g. "internal:confidential".
pub fn label_key(category: DataCategory, sensitivity: SensitivityLevel) -> String {
    let name = |v: serde_json::Value| v.as_str().unwrap_or_default().to_string();
    format!(
        "{}:{}",
        name(serde_json::to_value(category).unwrap_or_default()),
        name(serde_json::to_value(sensitivity).unwrap_or_default())
    )
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    vector: QuantizedVector,
    category: DataCategory,
    sensitivity: SensitivityLevel,
    family: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SemanticWire", into = "SemanticWire")]
pub struct SemanticIndex {
    pub format_version: String,
    pub taxonomy_version: String,
    pub embedder: EmbedderRecord,
    pub dimensions: usize,
    entries: Vec<Entry>,
    anchors: Vec<QuantizedVector>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticMatch {
    pub category: DataCategory,
    pub sensitivity: SensitivityLevel,
    pub family: String,
    pub margin: f32,
}

/// Builds a `SemanticIndex` from labelled protected documents and public anchors.
///
/// Anchors are never labelled and can never fire on their own — they exist
/// only to be subtracted from a candidate's cosine so that in-domain-but-benign
/// text (e.g. any drug leaflet, for a healthcare corpus) does not read as a
/// match.
pub struct SemanticIndexBuilder {
    taxonomy_version: String,
    embedder: EmbedderRecord,
    dimensions: Option<usize>,
    entries: Vec<Entry>,
    anchors: Vec<QuantizedVector>,
}

impl SemanticIndexBuilder {
    pub fn new(taxonomy_version: &str, embedder: EmbedderRecord) -> Self {
        Self {
            taxonomy_version: taxonomy_version.to_string(),
            embedder,
            dimensions: None,
            entries: Vec::new(),
            anchors: Vec::new(),
        }
    }

    fn check_dimensions(&mut self, vectors: &[Vec<f32>]) -> Result<(), String> {
        for v in vectors {
            match self.dimensions {
                None => self.dimensions = Some(v.len()),
                Some(d) if d != v.len() => {
                    return Err(format!(
                        "embedder produced a vector of {} dimensions, expected {d}",
                        v.len()
                    ));
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    pub fn add_document(
        &mut self,
        embedder: &dyn Embedder,
        text: &str,
        category: DataCategory,
        sensitivity: SensitivityLevel,
        family: &str,
    ) -> Result<usize, String> {
        let vectors = embed_units(embedder, text)?;
        self.check_dimensions(&vectors)?;
        let count = vectors.len();
        for v in &vectors {
            self.entries.push(Entry {
                vector: QuantizedVector::quantize(v),
                category,
                sensitivity,
                family: family.to_string(),
            });
        }
        Ok(count)
    }

    pub fn add_anchor(&mut self, embedder: &dyn Embedder, text: &str) -> Result<usize, String> {
        let vectors = embed_units(embedder, text)?;
        self.check_dimensions(&vectors)?;
        let count = vectors.len();
        for v in &vectors {
            self.anchors.push(QuantizedVector::quantize(v));
        }
        Ok(count)
    }

    pub fn build(self) -> Result<SemanticIndex, String> {
        if self.entries.is_empty() {
            return Err("a semantic index needs at least one protected unit".to_string());
        }
        if self.anchors.is_empty() {
            return Err(
                "a semantic index needs public anchors; without them every in-domain prompt fires"
                    .to_string(),
            );
        }
        Ok(SemanticIndex {
            format_version: SEMANTIC_FORMAT_VERSION.to_string(),
            taxonomy_version: self.taxonomy_version,
            embedder: self.embedder,
            dimensions: self.dimensions.unwrap_or_default(),
            entries: self.entries,
            anchors: self.anchors,
        })
    }
}

impl SemanticIndex {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn labels(&self) -> BTreeSet<String> {
        self.entries
            .iter()
            .map(|e| label_key(e.category, e.sensitivity))
            .collect()
    }

    pub fn max_sensitivity(&self) -> SensitivityLevel {
        self.entries
            .iter()
            .map(|e| e.sensitivity)
            .max()
            .unwrap_or(SensitivityLevel::Public)
    }

    fn label_of(&self, key: &str) -> Option<(DataCategory, SensitivityLevel)> {
        self.entries
            .iter()
            .find(|e| label_key(e.category, e.sensitivity) == key)
            .map(|e| (e.category, e.sensitivity))
    }

    fn best_anchor(&self, unit: &[f32]) -> f32 {
        self.anchors
            .iter()
            .map(|a| a.cosine(unit))
            .fold(f32::NEG_INFINITY, f32::max)
    }

    /// Best margin per label for one query unit: (label key → (margin, family)).
    pub fn margins(&self, unit: &[f32]) -> BTreeMap<String, (f32, String)> {
        let anchor = self.best_anchor(unit);
        let mut best: BTreeMap<String, (f32, String)> = BTreeMap::new();
        for entry in &self.entries {
            let key = label_key(entry.category, entry.sensitivity);
            let margin = entry.vector.cosine(unit) - anchor;
            let slot = best
                .entry(key)
                .or_insert((f32::NEG_INFINITY, String::new()));
            if margin > slot.0 {
                *slot = (margin, entry.family.clone());
            }
        }
        best
    }

    /// The strongest firing label over all units, or None.
    pub fn judge(
        &self,
        units: &[Vec<f32>],
        thresholds: &BTreeMap<String, f32>,
    ) -> Option<SemanticMatch> {
        let mut strongest: Option<SemanticMatch> = None;
        for unit in units {
            for (key, (margin, family)) in self.margins(unit) {
                let Some(threshold) = thresholds.get(&key) else {
                    continue;
                };
                if margin < *threshold {
                    continue;
                }
                let Some((category, sensitivity)) = self.label_of(&key) else {
                    continue;
                };
                let candidate = SemanticMatch {
                    category,
                    sensitivity,
                    family,
                    margin,
                };
                let stronger = strongest.as_ref().is_none_or(|s| {
                    (candidate.sensitivity, candidate.margin) > (s.sensitivity, s.margin)
                });
                if stronger {
                    strongest = Some(candidate);
                }
            }
        }
        strongest
    }
}

#[derive(Serialize, Deserialize)]
struct SemanticWire {
    format_version: String,
    taxonomy_version: String,
    embedder: EmbedderRecord,
    dimensions: usize,
    /// Base64 of `dimensions × i8` per entry, concatenated.
    vectors: String,
    labels: Vec<(DataCategory, SensitivityLevel)>,
    families: Vec<String>,
    anchors: String,
}

fn pack_vectors<'a>(vectors: impl Iterator<Item = &'a QuantizedVector>) -> String {
    let mut bytes = Vec::new();
    for v in vectors {
        bytes.extend(v.values.iter().map(|b| b.cast_unsigned()));
    }
    STANDARD.encode(bytes)
}

fn unpack_vectors(encoded: &str, dimensions: usize) -> Result<Vec<QuantizedVector>, String> {
    if dimensions == 0 {
        return Err("a semantic index cannot have zero dimensions".to_string());
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|e| format!("vector block is not valid base64: {e}"))?;
    if bytes.len() % dimensions != 0 {
        return Err(format!(
            "vector block of {} bytes is not a multiple of {dimensions} dimensions",
            bytes.len()
        ));
    }
    Ok(bytes
        .chunks_exact(dimensions)
        .map(|chunk| QuantizedVector {
            values: chunk.iter().map(|b| b.cast_signed()).collect(),
        })
        .collect())
}

impl From<SemanticIndex> for SemanticWire {
    fn from(idx: SemanticIndex) -> Self {
        let vectors = pack_vectors(idx.entries.iter().map(|e| &e.vector));
        let anchors = pack_vectors(idx.anchors.iter());
        let labels = idx
            .entries
            .iter()
            .map(|e| (e.category, e.sensitivity))
            .collect();
        let families = idx.entries.into_iter().map(|e| e.family).collect();
        SemanticWire {
            format_version: idx.format_version,
            taxonomy_version: idx.taxonomy_version,
            embedder: idx.embedder,
            dimensions: idx.dimensions,
            vectors,
            labels,
            families,
            anchors,
        }
    }
}

impl TryFrom<SemanticWire> for SemanticIndex {
    type Error = String;

    fn try_from(wire: SemanticWire) -> Result<Self, String> {
        if wire.format_version != SEMANTIC_FORMAT_VERSION {
            return Err(format!(
                "semantic format version {} is not supported (expected {SEMANTIC_FORMAT_VERSION})",
                wire.format_version
            ));
        }
        if wire.dimensions == 0 {
            return Err("a semantic index cannot have zero dimensions".to_string());
        }
        if wire.labels.len() != wire.families.len() {
            return Err(format!(
                "{} labels but {} families",
                wire.labels.len(),
                wire.families.len()
            ));
        }
        let vectors = unpack_vectors(&wire.vectors, wire.dimensions)?;
        if vectors.len() != wire.labels.len() {
            return Err(format!(
                "vector block decodes to {} vectors, expected {} (one per label)",
                vectors.len(),
                wire.labels.len()
            ));
        }
        let anchors = unpack_vectors(&wire.anchors, wire.dimensions)?;
        let entries = vectors
            .into_iter()
            .zip(wire.labels)
            .zip(wire.families)
            .map(|((vector, (category, sensitivity)), family)| Entry {
                vector,
                category,
                sensitivity,
                family,
            })
            .collect();
        Ok(SemanticIndex {
            format_version: wire.format_version,
            taxonomy_version: wire.taxonomy_version,
            embedder: wire.embedder,
            dimensions: wire.dimensions,
            entries,
            anchors,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::tests::BagOfWords;
    use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};

    fn record() -> EmbedderRecord {
        EmbedderRecord {
            source: "org/model/file.gguf".into(),
            sha256: "test-digest".into(),
            pooling: EmbeddingPooling::Last,
        }
    }

    const SECRET: &str = "the northwind acquisition closes in march with a hidden indemnity clause";
    const PUBLIC: &str = "oxycodone prescribing information warns of addiction abuse and misuse";

    fn index() -> SemanticIndex {
        let mut b = SemanticIndexBuilder::new("1.0.0", record());
        b.add_document(
            &BagOfWords,
            SECRET,
            DataCategory::Internal,
            SensitivityLevel::Confidential,
            "board",
        )
        .unwrap();
        b.add_document(
            &BagOfWords,
            "quarterly payroll ledger for the finance team",
            DataCategory::Financial,
            SensitivityLevel::Restricted,
            "fin",
        )
        .unwrap();
        b.add_anchor(&BagOfWords, PUBLIC).unwrap();
        b.build().unwrap()
    }

    fn units(text: &str) -> Vec<Vec<f32>> {
        crate::embed::embed_units(&BagOfWords, text).unwrap()
    }

    #[test]
    fn a_label_fires_when_its_margin_clears_the_threshold() {
        let idx = index();
        let t = BTreeMap::from([(
            label_key(DataCategory::Internal, SensitivityLevel::Confidential),
            0.3,
        )]);
        let hit = idx.judge(&units(SECRET), &t).expect("fires");
        assert_eq!(hit.sensitivity, SensitivityLevel::Confidential);
        assert_eq!(hit.family, "board");
        assert!(
            idx.judge(&units("an unrelated sentence about gardening"), &t)
                .is_none()
        );
    }

    #[test]
    fn anchors_subtract_and_never_fire_on_their_own() {
        let idx = index();
        let t = BTreeMap::from([(
            label_key(DataCategory::Internal, SensitivityLevel::Confidential),
            0.0,
        )]);
        assert!(
            idx.judge(&units(PUBLIC), &t).is_none(),
            "anchor text is not a finding"
        );
    }

    #[test]
    fn the_highest_sensitivity_that_fires_wins() {
        let idx = index();
        let t = BTreeMap::from([
            (
                label_key(DataCategory::Internal, SensitivityLevel::Confidential),
                -1.0,
            ),
            (
                label_key(DataCategory::Financial, SensitivityLevel::Restricted),
                -1.0,
            ),
        ]);
        let hit = idx.judge(&units(SECRET), &t).unwrap();
        assert_eq!(hit.sensitivity, SensitivityLevel::Restricted);
    }

    #[test]
    fn padding_with_benign_prose_cannot_dilute_a_match() {
        let idx = index();
        let t = BTreeMap::from([(
            label_key(DataCategory::Internal, SensitivityLevel::Confidential),
            0.3,
        )]);
        // One run-on span with no sentence punctuation and more than UNIT_WORDS
        // words on each side, so the chunker's long-sentence path (`split_long`)
        // flushes padding into units of its own rather than window-packing it
        // alongside SECRET. This tests cross-unit dilution (what `judge`'s max-
        // over-units is meant to resist), not within-unit dilution, which is
        // bounded by the unit size and is a measurement concern, not this test's
        // claim.
        let padding = "the weather was pleasant and the garden grew well ".repeat(30);
        let text = format!("{padding}. {SECRET}. {padding}");
        let chunks = crate::embed::semantic_units(&text);
        let secret_chunk = chunks
            .iter()
            .find(|u| u.to_lowercase().contains("northwind"));
        assert!(
            secret_chunk.is_some_and(|u| !u.to_lowercase().contains("weather")),
            "setup assumption: SECRET must land in a unit with no padding words, \
             otherwise this test exercises within-unit dilution instead of cross-unit"
        );
        assert!(idx.judge(&units(&text), &t).is_some());
    }

    #[test]
    fn a_label_without_a_threshold_never_fires() {
        let idx = index();
        assert!(idx.judge(&units(SECRET), &BTreeMap::new()).is_none());
    }

    #[test]
    fn the_index_round_trips_and_stores_no_corpus_words() {
        let idx = index();
        let json = serde_json::to_string(&idx).unwrap();
        for word in [
            "northwind",
            "acquisition",
            "indemnity",
            "payroll",
            "oxycodone",
        ] {
            assert!(
                !json.to_lowercase().contains(word),
                "{word} leaked into the index"
            );
        }
        let back: SemanticIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(back, idx);
    }

    #[test]
    fn a_build_without_anchors_or_entries_is_refused() {
        let mut b = SemanticIndexBuilder::new("1.0.0", record());
        b.add_document(
            &BagOfWords,
            SECRET,
            DataCategory::Internal,
            SensitivityLevel::Confidential,
            "board",
        )
        .unwrap();
        assert!(b.build().is_err(), "no anchors");
        let mut b = SemanticIndexBuilder::new("1.0.0", record());
        b.add_anchor(&BagOfWords, PUBLIC).unwrap();
        assert!(b.build().is_err(), "no entries");
    }

    #[test]
    fn a_truncated_vector_block_is_refused_on_load() {
        let mut v: serde_json::Value = serde_json::to_value(index()).unwrap();
        v["vectors"] = serde_json::Value::String("AAAA".into());
        assert!(serde_json::from_value::<SemanticIndex>(v).is_err());
    }
}
