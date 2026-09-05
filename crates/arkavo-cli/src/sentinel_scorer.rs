//! The distilled sentinel detector as a [`ScoringModel`] (SENT-002).
//!
//! `arkavo-sentinel` deliberately holds no inference code, so the GGUF-backed
//! classifier lives here, where the llama.cpp runtime already is. A node that
//! runs only the reference tiers links none of this.
//!
//! The scorer is a *single forward pass*, not a generation: the detector was
//! fine-tuned to answer with one word, so the label is read straight out of the
//! next-token distribution over the three label tokens. That is what
//! `scripts/distill/eval.py` does when it calibrates the thresholds, and the
//! two must agree token for token or the thresholds describe a different
//! detector than the one running.

use std::ffi::CString;
use std::path::Path;
use std::sync::Mutex;

use arkavo_llama_cpp::{
    ChatInputs, ChatTemplates, LlamaContext, LlamaModel, batch_get_one_with_logits, decode_batch,
    ffi, init_llama_logging, tokenize_with_model,
};
use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
use arkavo_sentinel::{RawLabel, ScoringModel};

/// The system prompt the detector was fine-tuned under (`SYSTEM` in
/// `scripts/distill/train.py`). Changing a character here silently changes the
/// distribution the calibration table was measured against.
pub const SENTINEL_SYSTEM: &str = "You are the Arkavo sentinel for this knowledge pack. \
Classify the user's text. Reply with exactly one word: public, internal, or confidential.";

/// The generation prompt the GGUF's own chat template emits for a
/// non-thinking assistant turn, and the tail every rendered prompt ends with.
const GENERATION_PROMPT: &str = "<|im_start|>assistant\n<think>\n\n</think>\n\n";

/// The taxonomy the detector was trained against, in the order `eval.py`
/// softmaxes the three logits. `confidential` and `internal` are both internal
/// data; only the sensitivity separates them.
const LABELS: [(&str, SensitivityLevel, DataCategory); 3] = [
    ("public", SensitivityLevel::Public, DataCategory::Public),
    (
        "internal",
        SensitivityLevel::Internal,
        DataCategory::Internal,
    ),
    (
        "confidential",
        SensitivityLevel::Confidential,
        DataCategory::Internal,
    ),
];

/// Maximum prompt size; longer content is inspected in overlapping spans.
const SCORING_CONTEXT_TOKENS: u32 = 1024;

/// A llama.cpp-backed classifier over a fine-tuned sentinel GGUF.
pub struct LlamaScoringModel {
    // Declaration order is drop order, and both the context and the templates
    // reference the model's C objects, so the model is declared last.
    context: Mutex<LlamaContext>,
    templates: ChatTemplates,
    model: LlamaModel,
    /// First token of each label in `LABELS` order — the positions read out of
    /// the next-token logits.
    label_tokens: [i32; 3],
    prompt_budget: usize,
    detector_version: String,
    taxonomy_version: String,
}

impl LlamaScoringModel {
    /// Load the sentinel GGUF.
    ///
    /// The versions are the caller's, not the file's: they come from the
    /// calibration table so that [`arkavo_sentinel::SentinelTier`]'s pairing
    /// check compares a real detector against the table that calibrated it.
    pub fn load(
        path: &Path,
        detector_version: &str,
        taxonomy_version: &str,
    ) -> Result<Self, String> {
        // Quiets llama.cpp's load-time chatter, which would otherwise bury the
        // caller's own output on stderr.
        init_llama_logging();
        let path_str = path
            .to_str()
            .ok_or_else(|| format!("sentinel model path is not UTF-8: {}", path.display()))?;
        let model = LlamaModel::from_file(path_str)?;
        let templates = model.chat_templates()?;
        let label_tokens = Self::label_tokens(&model)?;

        // The wrapper sizes the context from the model's metadata, so the
        // budget is clamped rather than assumed: it must never exceed the
        // context that was actually created.
        let ctx_tokens = SCORING_CONTEXT_TOKENS.min(model.get_trained_context_size());
        let prompt_budget = (ctx_tokens as usize).saturating_sub(1);
        if prompt_budget == 0 {
            return Err("sentinel model reports no usable context".to_string());
        }
        let context = LlamaContext::new(&model)?;

        Ok(Self {
            context: Mutex::new(context),
            templates,
            model,
            label_tokens,
            prompt_budget,
            detector_version: detector_version.to_string(),
            taxonomy_version: taxonomy_version.to_string(),
        })
    }

    /// Resolve the three label tokens once, at load.
    ///
    /// `eval.py` reads `tok.encode(label)[0]`; a multi-token label is scored on
    /// its first token, which is the position the distribution actually
    /// decides at.
    fn label_tokens(model: &LlamaModel) -> Result<[i32; 3], String> {
        let vocab = model.get_vocab();
        let n_vocab = model.n_vocab();
        let mut tokens = [0i32; 3];
        for (slot, (label, _, _)) in tokens.iter_mut().zip(LABELS) {
            let encoded = tokenize_with_model(vocab, label.as_bytes())?;
            let first = *encoded
                .first()
                .ok_or_else(|| format!("label `{label}` tokenized to nothing"))?;
            if first < 0 || first >= n_vocab {
                return Err(format!(
                    "label `{label}` tokenized to {first}, outside a vocabulary of {n_vocab}"
                ));
            }
            *slot = first;
        }
        Ok(tokens)
    }

    /// The prompt for one span, as bytes ready to tokenize.
    ///
    /// The GGUF's own chat template is the source of truth; the hand-formatted
    /// ChatML is the fallback for a span the template engine refuses (an
    /// interior NUL, most plausibly), so that a hostile span degrades the
    /// rendering rather than the classification.
    fn prompt_for(&self, text: &str) -> Vec<u8> {
        match self.render_with_template(text) {
            Ok(prompt) => prompt,
            Err(reason) => {
                tracing::warn!(%reason, "sentinel chat template unavailable; using ChatML");
                chatml_prompt(text).into_bytes()
            }
        }
    }

    fn render_with_template(&self, text: &str) -> Result<Vec<u8>, String> {
        let system = CString::new(SENTINEL_SYSTEM)
            .map_err(|e| format!("sentinel system prompt is not a C string: {e}"))?;
        let user = CString::new(text).map_err(|e| format!("span is not a C string: {e}"))?;
        let messages = [
            ffi::llama_chat_message {
                role: c"system".as_ptr(),
                content: system.as_ptr(),
            },
            ffi::llama_chat_message {
                role: c"user".as_ptr(),
                content: user.as_ptr(),
            },
        ];
        let inputs = ChatInputs {
            add_generation_prompt: true,
            // The template emits an empty `<think></think>` block unless
            // thinking is explicitly on, which is what training saw.
            enable_thinking: false,
            ..ChatInputs::default()
        };
        let rendered = self.templates.apply(&messages, &inputs)?;
        Ok(rendered.prompt)
    }

    fn score_inner(&self, text: &str) -> Result<Vec<RawLabel>, String> {
        score_spans(
            text,
            self.prompt_budget,
            |span| tokenize_with_model(self.model.get_vocab(), &self.prompt_for(span)),
            |tokens| self.score_tokens(tokens),
        )
        .map(raw_labels)
    }

    fn score_tokens(&self, tokens: &[i32]) -> Result<[f32; 3], String> {
        let ctx = self
            .context
            .lock()
            .map_err(|_| "sentinel context mutex poisoned".to_string())?;
        // `clear_kv_cache()` is a no-op in the wrapper; the memory handle is the
        // real reset, and an empty cache is what puts this prompt at position 0
        // of a single sequence — no state from the last span.
        ctx.get_memory().clear(true);
        let batch = batch_get_one_with_logits(tokens, true);
        decode_batch(&ctx, batch)?;
        // Read under the same guard: the logits row belongs to the decode above
        // and is only meaningful until the next one. Released as soon as the
        // three values are copied out, so the softmax is not serialized.
        let logits = label_logits(&ctx, self.label_tokens, self.model.n_vocab())?;
        drop(ctx);

        if logits.iter().any(|value| !value.is_finite()) {
            return Err("sentinel decode produced non-finite logits".into());
        }
        Ok(softmax(logits))
    }
}

impl ScoringModel for LlamaScoringModel {
    fn detector_version(&self) -> &str {
        &self.detector_version
    }

    fn taxonomy_version(&self) -> &str {
        &self.taxonomy_version
    }

    fn score(&self, text: &str) -> Result<Vec<RawLabel>, String> {
        self.score_inner(text)
    }
}

/// Preserve the entire prompt around each span and overlap by a quarter of
/// its characters so a boundary does not erase a classification signal.
fn score_spans(
    text: &str,
    budget: usize,
    tokenize: impl Fn(&str) -> Result<Vec<i32>, String>,
    infer: impl Fn(&[i32]) -> Result<[f32; 3], String>,
) -> Result<[f32; 3], String> {
    let mut scores = [0.0_f32; 3];
    let mut start: usize = 0;
    loop {
        let mut end = text
            .len()
            .min(start.saturating_add(budget.saturating_mul(4)));
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let tokens = loop {
            let tokens = tokenize(&text[start..end])?;
            if tokens.is_empty() {
                return Err("sentinel prompt tokenized to nothing".into());
            }
            if tokens.len() <= budget {
                break tokens;
            }
            let characters = text[start..end].chars().count();
            if characters <= 1 {
                return Err("sentinel context cannot fit the prompt and one character".into());
            }
            end = start
                + text[start..end]
                    .char_indices()
                    .nth(characters / 2)
                    .expect("half of a nonempty span")
                    .0;
        };
        for (maximum, score) in scores.iter_mut().zip(infer(&tokens)?) {
            *maximum = maximum.max(score);
        }
        if end == text.len() {
            return Ok(scores);
        }
        let characters = text[start..end].chars().count();
        if characters == 0 {
            return Err("sentinel context cannot fit one character".into());
        }
        let advance = characters - characters / 4;
        start = text[start..end]
            .char_indices()
            .nth(advance)
            .map_or(end, |(offset, _)| start + offset);
    }
}

/// The three label logits from the last decoded position.
///
/// Takes the context by reference so the caller's lock guard is what bounds the
/// pointer's validity.
fn label_logits(ctx: &LlamaContext, tokens: [i32; 3], n_vocab: i32) -> Result<[f32; 3], String> {
    let row = ctx.get_logits_ith(-1);
    if row.is_null() {
        return Err("sentinel decode produced no logits".to_string());
    }
    let mut logits = [0f32; 3];
    for (slot, token) in logits.iter_mut().zip(tokens) {
        if token < 0 || token >= n_vocab {
            return Err(format!("label token {token} outside vocabulary {n_vocab}"));
        }
        // SAFETY: `row` is the n_vocab-wide logits row for the last decoded
        // token, valid for as long as the caller holds the context lock, and
        // `token` is checked against that width.
        *slot = unsafe { *row.add(token as usize) };
    }
    Ok(logits)
}

/// The training-time ChatML rendering, used when the template engine cannot
/// render a span.
fn chatml_prompt(text: &str) -> String {
    format!(
        "<|im_start|>system\n{SENTINEL_SYSTEM}<|im_end|>\n<|im_start|>user\n{text}<|im_end|>\n{GENERATION_PROMPT}"
    )
}

/// Temperature-1 softmax over the three label logits only, matching `eval.py`.
///
/// Accumulated in f64: the three logits can differ by tens of nats, which is
/// where an f32 exponential loses the smaller two entirely.
fn softmax(values: [f32; 3]) -> [f32; 3] {
    let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if !max.is_finite() {
        // A degenerate row expresses no preference, and a uniform split says
        // exactly that rather than inventing one.
        return [1.0 / 3.0; 3];
    }
    let exps = values.map(|v| f64::from(v - max).exp());
    let sum: f64 = exps.iter().sum();
    if !sum.is_finite() || sum <= 0.0 {
        return [1.0 / 3.0; 3];
    }
    exps.map(|e| (e / sum) as f32)
}

/// The three probabilities as raw labels, in `LABELS` order.
fn raw_labels(probs: [f32; 3]) -> Vec<RawLabel> {
    LABELS
        .iter()
        .zip(probs)
        .map(|((label, sensitivity, category), score)| RawLabel {
            label: (*label).to_string(),
            category: *category,
            sensitivity: *sensitivity,
            score,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A holiday-schedule notice: unremarkable on its own, but internal
    /// distribution — the case that separates `internal` from `public`.
    const HOLIDAY_NOTICE: &str = "From: Human Resources  To: All Employees  \
Subject: Holiday schedule and cafeteria hours. The offices will be closed Monday, \
September 1 for Labor Day and Thursday, November 27 for Thanksgiving. The cafeteria \
will close at 2:00 PM on the Wednesday before each holiday and reopen on the following \
business day. Please submit any coverage requests to your manager.";

    const CONFIDENTIAL_SPAN: &str = "From: Decker, John F  To: Baker, Michael  Subject: RE: \
Dr. Hausrod literature review invoice. Michael, please have Dr. Hausrod bill you directly \
and pass the invoice through to us as an expense. Thanks, John. MNKOI 0001599301";

    const PUBLIC_SPAN: &str = "Acetaminophen is a common over-the-counter analgesic and \
antipyretic. The FDA recommends a maximum daily dose of 4 grams for adults; liver injury \
has been reported with overdose.";

    /// A second corpus-shaped span: distribution limits and a Bates number, the
    /// marks that make a document confidential rather than merely internal.
    const RESTRICTED_MEMO: &str = "From: Harper, Susan  To: Regional Sales Directors  \
Subject: Q3 detailing targets. Attached are the individual call quotas and the physician \
target list; do not circulate outside the sales leadership group. MNKOI 0002214477";

    fn gguf_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("models/oida-qa/mallinckrodt/sentinel-qwen3.5-0.8b-mallinckrodt.gguf")
    }

    fn argmax(labels: &[RawLabel]) -> &str {
        &labels
            .iter()
            .max_by(|a, b| a.score.total_cmp(&b.score))
            .expect("three labels")
            .label
    }

    #[test]
    fn long_spans_preserve_prefix_overlap_and_utf8() {
        let text = format!("secret{}tail", "é日x".repeat(30));
        let seen = std::cell::RefCell::new(Vec::new());
        let scores = score_spans(
            &text,
            20,
            |span| {
                seen.borrow_mut().push(span.to_string());
                // Reserve prompt overhead exactly as the real tokenizer does.
                Ok(std::iter::repeat_n(0, 8)
                    .chain(span.chars().map(|c| c as i32))
                    .collect())
            },
            |tokens| {
                let span: String = tokens[8..]
                    .iter()
                    .map(|&c| char::from_u32(c as u32).unwrap())
                    .collect();
                Ok(if span.contains("secret") {
                    [0.0, 0.0, 1.0]
                } else {
                    [1.0, 0.0, 0.0]
                })
            },
        )
        .unwrap();
        assert_eq!(scores[2], 1.0, "prefix finding survives the public tail");
        let inspected: Vec<_> = seen
            .borrow()
            .iter()
            .filter(|span| span.chars().count() + 8 <= 20)
            .cloned()
            .collect();
        assert!(inspected.first().unwrap().starts_with("secret"));
        assert!(inspected.last().unwrap().ends_with("tail"));
        let mut rebuilt = inspected[0].clone();
        for pair in inspected.windows(2) {
            let overlap = pair[0].chars().count() / 4;
            let suffix: String = pair[0]
                .chars()
                .skip(pair[0].chars().count() - overlap)
                .collect();
            assert!(pair[1].starts_with(&suffix));
            rebuilt.extend(pair[1].chars().skip(overlap));
        }
        assert_eq!(rebuilt, text);
    }

    #[test]
    fn failures_and_uninspectable_context_are_errors() {
        assert!(
            score_spans(
                "text",
                20,
                |_| Err("tokenization failed".into()),
                |_| Ok([0.0; 3])
            )
            .is_err()
        );
        assert!(score_spans("text", 20, |_| Ok(vec![0]), |_| Err("decode failed".into())).is_err());
        assert!(score_spans("text", 1, |_| Ok(vec![0; 8]), |_| Ok([0.0; 3])).is_err());
        assert!(score_spans("text", 20, |_| Ok(vec![]), |_| Ok([0.0; 3])).is_err());
    }

    #[test]
    fn a_later_segment_failure_discards_partial_success() {
        let calls = std::cell::Cell::new(0);
        let result = score_spans(
            "abcdefghijklmnop",
            8,
            |span| Ok(span.chars().map(|c| c as i32).collect()),
            |_| {
                calls.set(calls.get() + 1);
                if calls.get() == 1 {
                    Ok([1.0, 0.0, 0.0])
                } else {
                    Err("later decode failed".into())
                }
            },
        );
        assert_eq!(result.unwrap_err(), "later decode failed");
    }

    #[test]
    fn softmax_is_a_distribution() {
        let probs = softmax([1.0, 2.0, 3.0]);
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6, "probabilities sum to {sum}");
        assert!(probs[0] < probs[1] && probs[1] < probs[2]);
        // exp(1)/(exp(1)+exp(2)+exp(3)) — the reference value eval.py computes.
        assert!((probs[0] - 0.090_030_57).abs() < 1e-6, "{probs:?}");
    }

    #[test]
    fn softmax_survives_a_wide_spread() {
        let probs = softmax([-60.0, 40.0, -1.0]);
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6, "probabilities sum to {sum}");
        assert!(probs[1] > 0.999);
    }

    #[test]
    fn softmax_of_a_degenerate_row_is_uniform() {
        let probs = softmax([f32::NEG_INFINITY; 3]);
        for p in probs {
            assert!((p - 1.0 / 3.0).abs() < 1e-6, "{probs:?}");
        }
    }

    #[test]
    fn labels_map_onto_the_taxonomy() {
        let labels = raw_labels([0.1, 0.2, 0.7]);
        let mapped: Vec<_> = labels
            .iter()
            .map(|l| (l.label.as_str(), l.sensitivity, l.category, l.score))
            .collect();
        assert_eq!(
            mapped,
            vec![
                (
                    "public",
                    SensitivityLevel::Public,
                    DataCategory::Public,
                    0.1
                ),
                (
                    "internal",
                    SensitivityLevel::Internal,
                    DataCategory::Internal,
                    0.2
                ),
                (
                    "confidential",
                    SensitivityLevel::Confidential,
                    DataCategory::Internal,
                    0.7
                ),
            ]
        );
    }

    #[test]
    fn chatml_fallback_matches_the_training_rendering() {
        let prompt = chatml_prompt("hello");
        assert!(prompt.ends_with(GENERATION_PROMPT), "{prompt}");
        assert_eq!(
            prompt,
            format!(
                "<|im_start|>system\n{SENTINEL_SYSTEM}<|im_end|>\n\
                 <|im_start|>user\nhello<|im_end|>\n\
                 <|im_start|>assistant\n<think>\n\n</think>\n\n"
            )
        );
    }

    /// The real detector. Skipped where the GGUF is not present, which is every
    /// checkout that has not fetched the example pack.
    #[test]
    fn scores_the_probe_spans() {
        let path = gguf_path();
        if !path.is_file() {
            eprintln!("skipping: no sentinel GGUF at {}", path.display());
            return;
        }
        let model = LlamaScoringModel::load(&path, "qwen3.5-0.8b-mallinckrodt-lora", "1.0.0")
            .expect("load sentinel");
        assert_eq!(model.detector_version(), "qwen3.5-0.8b-mallinckrodt-lora");
        assert_eq!(model.taxonomy_version(), "1.0.0");

        // The label positions eval.py reads. A pre-tokenizer disagreement
        // between transformers and llama.cpp would show up here first.
        assert_eq!(model.label_tokens, [860, 10168, 5943]);

        let prompt = model.prompt_for(PUBLIC_SPAN);
        let rendered = String::from_utf8(prompt.clone()).expect("prompt is UTF-8");
        assert!(
            rendered.ends_with(GENERATION_PROMPT),
            "rendered prompt tail: {:?}",
            &rendered[rendered.len().saturating_sub(64)..]
        );
        assert!(rendered.starts_with("<|im_start|>system\n"), "{rendered}");

        // No BOS: training tokenized with `add_special_tokens=False`, and the
        // first token must therefore be the template's own `<|im_start|>`.
        let tokens = tokenize_with_model(model.model.get_vocab(), &prompt).expect("tokenize");
        let first = arkavo_llama_cpp::token_to_piece(model.model.get_vocab(), tokens[0], true)
            .expect("detokenize");
        assert_eq!(first, "<|im_start|>");

        for (span, expected) in [
            (CONFIDENTIAL_SPAN, "confidential"),
            (PUBLIC_SPAN, "public"),
            (HOLIDAY_NOTICE, "internal"),
            (RESTRICTED_MEMO, "confidential"),
        ] {
            let labels = model.score(span).expect("score span");
            assert_eq!(labels.len(), 3);
            let sum: f32 = labels.iter().map(|l| l.score).sum();
            assert!((sum - 1.0).abs() < 1e-4, "scores sum to {sum}");
            let scores: Vec<_> = labels
                .iter()
                .map(|l| format!("{}={:.3e}", l.label, l.score))
                .collect();
            eprintln!("[{}] {}", argmax(&labels), scores.join(" "));
            assert_eq!(
                argmax(&labels),
                expected,
                "span starting {:?} scored {:?}",
                &span[..40.min(span.len())],
                scores
            );
        }
    }
}
