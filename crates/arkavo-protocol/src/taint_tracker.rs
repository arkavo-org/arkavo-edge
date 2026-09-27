//! Session-scoped data taint tracking (SEQ-001, SEQ-002, SEQ-004).
//!
//! The tracker is the thing that knows, at any moment, what a buffer in this
//! session is carrying. It tags data where it enters, carries the tags through
//! transformations, and records both into the session's action graph so a
//! later gate can ask where a payload came from instead of only where it is
//! going.
//!
//! Every method takes `&self`. The tracker sits on the per-call path of a tool
//! loop that already holds the data behind an `Arc`, and requiring a write lock
//! there would put the gate's cost on every concurrent call rather than on the
//! one recording. The graph is the only mutable state, so it carries its own
//! lock.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use arkavo_events::EventPayload;
use serde_json::Value;

use crate::data_classification::SensitivityLevel;
use crate::sequence_graph::{GraphError, NodeId, SequenceGraphBuilder};
use crate::taint::{TaintLabel, TaintSet, TaintSource, Transformation};
use crate::taint_inference::{ClassificationInferencer, RegexInferencer};

/// Classification applied when nothing better is known.
///
/// `Internal` rather than `Public`: an unlabelled buffer in an agent session
/// came from somewhere, and treating unknown provenance as publishable is the
/// failure mode this whole subsystem exists to prevent (SEQ-001 edge case:
/// ambiguous source classification is conservative).
pub const DEFAULT_FLOOR: SensitivityLevel = SensitivityLevel::Internal;

/// Per-model classification ceilings.
///
/// A model that was trained or fine-tuned on classified material can emit that
/// material unprompted, so its output carries the ceiling whether or not the
/// request did. Phase 5 reads these from signed pack metadata; until then they
/// come from configuration, which is why an unknown model gets the
/// conservative default rather than `Public`.
#[derive(Debug, Clone)]
pub struct ModelCeilings {
    ceilings: BTreeMap<String, SensitivityLevel>,
    default: SensitivityLevel,
}

impl Default for ModelCeilings {
    fn default() -> Self {
        Self {
            ceilings: BTreeMap::new(),
            default: DEFAULT_FLOOR,
        }
    }
}

impl ModelCeilings {
    pub fn new(default: SensitivityLevel) -> Self {
        Self {
            ceilings: BTreeMap::new(),
            default,
        }
    }

    #[must_use]
    pub fn with(mut self, model_id: impl Into<String>, ceiling: SensitivityLevel) -> Self {
        self.ceilings.insert(model_id.into(), ceiling);
        self
    }

    pub fn ceiling_for(&self, model_id: &str) -> SensitivityLevel {
        self.ceilings.get(model_id).copied().unwrap_or(self.default)
    }
}

/// Tracks taint and action sequence for one session.
pub struct DataTaintTracker {
    session_id: String,
    inferencer: Arc<dyn ClassificationInferencer>,
    floor: SensitivityLevel,
    ceilings: ModelCeilings,
    graph: Mutex<SequenceGraphBuilder>,
}

impl DataTaintTracker {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            inferencer: Arc::new(RegexInferencer::new()),
            floor: DEFAULT_FLOOR,
            ceilings: ModelCeilings::default(),
            graph: Mutex::new(SequenceGraphBuilder::new()),
        }
    }

    /// Swap the classification tier. This is the SEQ-001 inference seam: the
    /// sentinel plugs in here without the tracker changing.
    #[must_use]
    pub fn with_inferencer(mut self, inferencer: Arc<dyn ClassificationInferencer>) -> Self {
        self.inferencer = inferencer;
        self
    }

    #[must_use]
    pub fn with_floor(mut self, floor: SensitivityLevel) -> Self {
        self.floor = floor;
        self
    }

    #[must_use]
    pub fn with_model_ceilings(mut self, ceilings: ModelCeilings) -> Self {
        self.ceilings = ceilings;
        self
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The graph lock, recovered rather than propagated if poisoned.
    ///
    /// The tracker sits on the per-call path of the conductor's tool loop.
    /// Propagating a poison there would turn one panicking writer into a
    /// session that panics on every later call — trading a forensics gap for a
    /// torn-down agent, which is the worse of the two. Recovery is sound
    /// because [`SequenceGraphBuilder::push`] validates before it mutates and
    /// appends the node before indexing it, so a poisoned graph holds a valid
    /// prefix of the session rather than a corrupt structure.
    fn graph_lock(&self) -> MutexGuard<'_, SequenceGraphBuilder> {
        self.graph.lock().unwrap_or_else(|poisoned| {
            tracing::warn!(
                session = %self.session_id,
                "sequence graph lock was poisoned; continuing from the recorded prefix"
            );
            poisoned.into_inner()
        })
    }

    /// Snapshot of the session's action graph.
    ///
    /// A clone rather than a borrow: the graph is behind a lock, and handing
    /// out a guard would let a caller hold it across an await on the tool path.
    pub fn graph(&self) -> SequenceGraphBuilder {
        self.graph_lock().clone()
    }

    pub fn inferencer_name(&self) -> &'static str {
        self.inferencer.name()
    }

    /// SEQ-001: tag data as it enters the session.
    ///
    /// The result is never empty. A source that declares itself `Public` is
    /// still labelled, because provenance stays useful after a classification
    /// turns out to have been wrong.
    pub fn ingest(&self, source: &TaintSource, text: &str) -> TaintSet {
        let found = self.inferencer.infer(text);
        // A declaration is the only thing that lowers the floor. Silence about
        // a source is not a claim that it is public.
        let floor = source.declared.unwrap_or(self.floor);
        TaintSet::from_label(TaintLabel::from_classifications(
            source.source_id(),
            &found,
            floor,
        ))
    }

    /// SEQ-001 edge case: data arriving from another agent inherits the
    /// upstream labels as well as whatever this hop can infer, so a chain of
    /// agents cannot launder a classification by relaying it.
    pub fn ingest_from_agent(
        &self,
        source: &TaintSource,
        text: &str,
        upstream: &TaintSet,
    ) -> TaintSet {
        self.ingest(source, text).union(upstream)
    }

    /// SEQ-002: output of a transformation inherits every input's taint and
    /// records what was done. Encoding is a hop, not an exit.
    pub fn transform(
        &self,
        inputs: &[&TaintSet],
        transformation: Transformation,
        detail: &str,
    ) -> TaintSet {
        let mut merged = TaintSet::new();
        for input in inputs {
            merged.merge(input);
        }
        merged.transformed(transformation, detail)
    }

    /// SEQ-002 edge case: output of inference is tainted by its input, and
    /// additionally by whatever the serving model itself may reveal.
    pub fn after_inference(&self, inputs: &[&TaintSet], model_id: &str) -> TaintSet {
        let ceiling = self.ceilings.ceiling_for(model_id);
        self.transform(inputs, Transformation::Inference, model_id)
            .raised_to(&format!("model:{model_id}"), ceiling)
    }

    /// SEQ-004: record a completed call. `inputs` names the nodes whose output
    /// reached this call; their taint is unioned in from the graph rather than
    /// taken on the caller's word.
    ///
    pub fn record_call(
        &self,
        tool_name: &str,
        params: &Value,
        inputs: &[NodeId],
        taint: &TaintSet,
    ) -> Result<NodeId, GraphError> {
        let mut graph = self.graph_lock();
        let carried = graph.taint_flowing_from(inputs)?.union(taint);
        graph.push(tool_name, params, inputs, carried)
    }

    /// The session's graph as ledger entries, oldest first.
    ///
    pub fn ledger_entries(&self) -> Vec<EventPayload> {
        self.graph_lock()
            .nodes()
            .iter()
            .map(|node| EventPayload::SequenceNode {
                node_id: node.id.clone(),
                tool_name: node.tool_name.clone(),
                params_hash: node.params_hash.clone(),
                inputs: node.inputs.clone(),
                taint: (&node.taint).into(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_classification::{ClassifiedDatum, DataCategory};
    use crate::taint::SourceKind;
    use serde_json::json;

    fn tool(name: &str) -> TaintSource {
        TaintSource::new(SourceKind::ToolResult, name)
    }

    #[test]
    fn ingestion_labels_even_benign_text() {
        let tracker = DataTaintTracker::new("s1");

        let set = tracker.ingest(&tool("read_file"), "the quick brown fox");

        assert!(!set.is_empty());
        assert_eq!(set.sensitivity(), SensitivityLevel::Internal);
    }

    #[test]
    fn ingestion_raises_to_what_the_detector_found() {
        let tracker = DataTaintTracker::new("s1");

        let set = tracker.ingest(&tool("read_file"), &format!("key {}", fake_api_key()));

        assert_eq!(set.sensitivity(), SensitivityLevel::Restricted);
        assert!(set.contains_category(DataCategory::Credentials));
    }

    #[test]
    fn a_declared_public_source_still_carries_a_label() {
        let tracker = DataTaintTracker::new("s1");
        let source = tool("fetch_docs").declared(SensitivityLevel::Public);

        let set = tracker.ingest(&source, "public documentation");

        assert_eq!(set.sensitivity(), SensitivityLevel::Public);
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn a_declared_public_source_cannot_hide_a_credential_in_its_content() {
        let tracker = DataTaintTracker::new("s1");
        let source = tool("fetch_docs").declared(SensitivityLevel::Public);

        let set = tracker.ingest(&source, &format!("example: {}", fake_api_key()));

        assert_eq!(set.sensitivity(), SensitivityLevel::Restricted);
    }

    #[test]
    fn agent_ingestion_inherits_upstream_labels() {
        let tracker = DataTaintTracker::new("s1");
        let upstream = tracker.ingest(&tool("vault"), &fake_api_key());
        let peer = TaintSource::new(SourceKind::A2aReceive, "did:key:z6Mk");

        let set = tracker.ingest_from_agent(&peer, "here is the summary", &upstream);

        assert_eq!(set.sensitivity(), SensitivityLevel::Restricted);
        assert!(set.contains_category(DataCategory::Credentials));
    }

    #[test]
    fn transformation_keeps_the_input_classification() {
        let tracker = DataTaintTracker::new("s1");
        let input = tracker.ingest(&tool("vault"), &fake_api_key());

        let encoded = tracker.transform(&[&input], Transformation::Encode, "base64");

        assert_eq!(encoded.sensitivity(), SensitivityLevel::Restricted);
    }

    #[test]
    fn combining_tainted_and_public_data_keeps_the_taint() {
        let tracker = DataTaintTracker::new("s1");
        let secret = tracker.ingest(&tool("vault"), &fake_api_key());
        let public = tracker.ingest(
            &tool("docs").declared(SensitivityLevel::Public),
            "hello world",
        );

        let merged = tracker.transform(&[&secret, &public], Transformation::Merge, "concat");

        assert_eq!(merged.sensitivity(), SensitivityLevel::Restricted);
    }

    #[test]
    fn inference_output_inherits_input_taint() {
        let tracker = DataTaintTracker::new("s1");
        let input = tracker.ingest(&tool("vault"), &fake_api_key());

        let output = tracker.after_inference(&[&input], "gemma-e2b");

        assert_eq!(output.sensitivity(), SensitivityLevel::Restricted);
    }

    #[test]
    fn inference_output_carries_the_model_ceiling_on_clean_input() {
        let tracker = DataTaintTracker::new("s1").with_model_ceilings(
            ModelCeilings::new(SensitivityLevel::Public)
                .with("internal-tuned", SensitivityLevel::Confidential),
        );

        let output = tracker.after_inference(&[], "internal-tuned");

        assert_eq!(output.sensitivity(), SensitivityLevel::Confidential);
    }

    #[test]
    fn an_unconfigured_model_gets_the_conservative_ceiling() {
        let tracker = DataTaintTracker::new("s1");

        let output = tracker.after_inference(&[], "never-configured");

        assert_eq!(output.sensitivity(), DEFAULT_FLOOR);
    }

    #[test]
    fn recorded_calls_inherit_taint_from_their_inputs() {
        let tracker = DataTaintTracker::new("s1");
        let secret = tracker.ingest(&tool("vault"), &fake_api_key());
        let read = tracker
            .record_call("read_vault", &json!({}), &[], &secret)
            .expect("root call");

        let post = tracker
            .record_call(
                "http_post",
                &json!({"url": "https://x"}),
                &[read],
                &TaintSet::new(),
            )
            .expect("known input");

        let graph = tracker.graph();
        let node = graph.node(&post).expect("node");
        assert_eq!(node.taint.sensitivity(), SensitivityLevel::Restricted);
    }

    #[test]
    fn ledger_entries_mirror_the_graph() {
        let tracker = DataTaintTracker::new("s1");
        let secret = tracker.ingest(&tool("vault"), &fake_api_key());
        tracker
            .record_call("read_vault", &json!({}), &[], &secret)
            .expect("root call");

        let entries = tracker.ledger_entries();

        assert_eq!(entries.len(), 1);
        match &entries[0] {
            EventPayload::SequenceNode {
                tool_name, taint, ..
            } => {
                assert_eq!(tool_name, "read_vault");
                assert_eq!(taint.sensitivity, "restricted");
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    struct SilentInferencer;

    impl ClassificationInferencer for SilentInferencer {
        fn name(&self) -> &'static str {
            "silent"
        }
        fn version(&self) -> &'static str {
            "0"
        }
        fn infer(&self, _text: &str) -> Vec<ClassifiedDatum> {
            Vec::new()
        }
    }

    #[test]
    fn a_detector_that_finds_nothing_cannot_clear_the_floor() {
        let tracker = DataTaintTracker::new("s1").with_inferencer(Arc::new(SilentInferencer));

        let set = tracker.ingest(&tool("read_file"), &fake_api_key());

        assert_eq!(tracker.inferencer_name(), "silent");
        assert_eq!(set.sensitivity(), DEFAULT_FLOOR);
    }

    /// Builds a credential-shaped string at run time.
    ///
    /// Generated rather than written down: a literal that matches a secret pattern
    /// trips scanners on every clone of this repo, and a scanner that cries wolf on
    /// fixtures is one people learn to ignore. The pieces are inert separately, and
    /// the value is deterministic so a failure stays reproducible.
    fn fake_api_key() -> String {
        let prefix: String = ['s', 'k'].iter().collect();
        let body: String = (0..24)
            .map(|i| char::from(b'a' + ((i * 7 + 3) % 26) as u8))
            .collect();
        format!("{prefix}-{body}")
    }
}
