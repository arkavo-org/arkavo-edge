//! Pooled sentence embeddings from a GGUF model.
//!
//! One context embeds many texts per forward pass: texts are packed into
//! batches as independent sequences, so a text's vector does not depend on
//! what it was batched with.

use crate::{ffi, LlamaContext, LlamaModel};
use std::ffi::CString;
use std::os::raw::c_char;

/// How per-token hidden states are reduced to one vector per text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolingType {
    Mean,
    Cls,
    Last,
}

impl PoolingType {
    fn to_ffi(self) -> ffi::llama_pooling_type {
        match self {
            Self::Mean => ffi::llama_pooling_type_LLAMA_POOLING_TYPE_MEAN,
            Self::Cls => ffi::llama_pooling_type_LLAMA_POOLING_TYPE_CLS,
            Self::Last => ffi::llama_pooling_type_LLAMA_POOLING_TYPE_LAST,
        }
    }

    /// `NONE`, `RANK` and `UNSPECIFIED` do not yield one vector per text, so
    /// they map to `None` rather than to a pooling the caller never asked for.
    fn from_ffi(code: i64) -> Option<Self> {
        match code {
            c if c == i64::from(ffi::llama_pooling_type_LLAMA_POOLING_TYPE_MEAN) => {
                Some(Self::Mean)
            }
            c if c == i64::from(ffi::llama_pooling_type_LLAMA_POOLING_TYPE_CLS) => Some(Self::Cls),
            c if c == i64::from(ffi::llama_pooling_type_LLAMA_POOLING_TYPE_LAST) => {
                Some(Self::Last)
            }
            _ => None,
        }
    }
}

/// Pooling the GGUF declares, if any (`<arch>.pooling_type`).
pub fn model_pooling(model: &LlamaModel) -> Option<PoolingType> {
    let arch = meta_str(model, "general.architecture")?;
    let code = meta_str(model, &format!("{arch}.pooling_type"))?;
    PoolingType::from_ffi(code.trim().parse().ok()?)
}

fn meta_str(model: &LlamaModel, key: &str) -> Option<String> {
    let key = CString::new(key).ok()?;
    let mut buf = vec![0u8; 256];
    // SAFETY: model.ptr is valid for the lifetime of LlamaModel; key is a
    // NUL-terminated CString and buf is writable for buf.len() bytes.
    let n = unsafe {
        ffi::llama_model_meta_val_str(
            model.ptr,
            key.as_ptr(),
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
        )
    };
    // A negative length means the key is absent; a length at or past the
    // buffer means the value was cut short, and a truncated value is not one
    // we can trust.
    let n = usize::try_from(n).ok().filter(|&n| n < buf.len())?;
    buf.truncate(n);
    String::from_utf8(buf).ok()
}

/// A llama.cpp context configured to return one pooled vector per sequence.
///
/// Holds a raw context, so it is `Send` but not `Sync`. The model it was
/// built from must outlive it, as with [`LlamaContext`].
pub struct EmbeddingContext {
    ctx: LlamaContext,
    model: *const ffi::llama_model,
    n_embd: usize,
    n_seq_max: usize,
}

// SAFETY: the context is only touched through `&mut self`, so moving it to
// another thread cannot race; the model pointer is only compared, never
// dereferenced.
unsafe impl Send for EmbeddingContext {}

impl EmbeddingContext {
    /// Tokens per batch, and the longest text accepted.
    pub const N_CTX: u32 = 2048;
    const N_SEQ_MAX: u32 = 16;

    pub fn new(model: &LlamaModel, pooling: PoolingType) -> Result<Self, String> {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8)
            .min(16) as i32;

        // SAFETY: returns a default-initialised struct by value.
        let mut params = unsafe { ffi::llama_context_default_params() };
        params.embeddings = true;
        params.pooling_type = pooling.to_ffi();
        params.n_ctx = Self::N_CTX;
        // Pooling over non-causal attention needs a whole sequence in one
        // micro-batch, so the micro-batch spans the full window.
        params.n_batch = Self::N_CTX;
        params.n_ubatch = Self::N_CTX;
        params.n_seq_max = Self::N_SEQ_MAX;
        // Without a unified cache llama.cpp splits n_ctx evenly across
        // sequences, capping each text at N_CTX / N_SEQ_MAX tokens.
        params.kv_unified = true;
        params.n_threads = threads;
        params.n_threads_batch = threads;

        // SAFETY: model.ptr was validated during LlamaModel construction.
        let ptr = unsafe { ffi::llama_new_context_with_model(model.ptr, params) };
        if ptr.is_null() {
            return Err("failed to create embedding context".to_string());
        }
        let ctx = LlamaContext { ptr };

        // SAFETY: model.ptr is valid; this reads a model hyperparameter. The
        // pooled vector is sized by the output width, which differs from
        // the hidden width for models with an output projection.
        let n_embd = unsafe { ffi::llama_model_n_embd_out(model.ptr) };
        let n_embd = usize::try_from(n_embd)
            .ok()
            .filter(|&n| n > 0)
            .ok_or_else(|| format!("model reports an invalid embedding width {n_embd}"))?;
        // SAFETY: ctx.ptr is non-null and owned by `ctx`.
        let n_seq_max = unsafe { ffi::llama_n_seq_max(ctx.ptr) } as usize;

        Ok(Self {
            ctx,
            model: model.ptr,
            n_embd,
            n_seq_max: n_seq_max.max(1),
        })
    }

    pub fn n_embd(&self) -> usize {
        self.n_embd
    }

    /// One pooled vector per text, in order. A text whose tokens exceed
    /// `N_CTX` is an error, never truncated.
    pub fn embed(&mut self, model: &LlamaModel, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        if !std::ptr::eq(model.ptr, self.model) {
            return Err("embed called with a model other than the context's own".to_string());
        }
        let tokenised = texts
            .iter()
            .enumerate()
            .map(|(i, text)| tokenise(model, text).map_err(|e| format!("text {i}: {e}")))
            .collect::<Result<Vec<_>, _>>()?;

        let mut out = Vec::with_capacity(texts.len());
        let mut start = 0;
        while start < tokenised.len() {
            let mut end = start;
            let mut total = 0;
            while end < tokenised.len()
                && end - start < self.n_seq_max
                && total + tokenised[end].len() <= Self::N_CTX as usize
            {
                total += tokenised[end].len();
                end += 1;
            }
            out.extend(self.embed_batch(model, &tokenised[start..end], total)?);
            start = end;
        }
        Ok(out)
    }

    fn embed_batch(
        &mut self,
        model: &LlamaModel,
        seqs: &[Vec<ffi::llama_token>],
        total: usize,
    ) -> Result<Vec<Vec<f32>>, String> {
        // SAFETY: ctx.ptr is valid; clearing a null memory handle (models
        // without a KV cache) is a no-op in llama.cpp.
        unsafe { ffi::llama_memory_clear(ffi::llama_get_memory(self.ctx.ptr), true) };

        let mut batch = Batch::new(total);
        let mut i = 0;
        for (seq_id, tokens) in seqs.iter().enumerate() {
            for (pos, &token) in tokens.iter().enumerate() {
                // SAFETY: llama_batch_init allocated every array for `total`
                // tokens with one seq_id slot each, and i < total because
                // the packer summed exactly these lengths into `total`.
                unsafe {
                    *batch.0.token.add(i) = token;
                    *batch.0.pos.add(i) = pos as i32;
                    *batch.0.n_seq_id.add(i) = 1;
                    *(*batch.0.seq_id.add(i)) = seq_id as i32;
                    // Every token is an output: mean pooling reads them all.
                    *batch.0.logits.add(i) = 1;
                }
                i += 1;
            }
        }
        batch.0.n_tokens = total as i32;

        // SAFETY: model.ptr is valid; these read model properties.
        let encoder_only = unsafe {
            ffi::llama_model_has_encoder(model.ptr) && !ffi::llama_model_has_decoder(model.ptr)
        };
        // SAFETY: ctx.ptr is valid and the batch is fully initialised; its
        // arrays stay alive until `batch` drops after this call.
        let rc = unsafe {
            if encoder_only {
                ffi::llama_encode(self.ctx.ptr, batch.0)
            } else {
                ffi::llama_decode(self.ctx.ptr, batch.0)
            }
        };
        if rc != 0 {
            return Err(format!("embedding forward pass failed (code {rc})"));
        }

        (0..seqs.len())
            .map(|seq_id| {
                // SAFETY: ctx.ptr is valid; the returned pointer is owned by
                // the context and valid until its next forward pass.
                let v = unsafe { ffi::llama_get_embeddings_seq(self.ctx.ptr, seq_id as i32) };
                if v.is_null() {
                    return Err(format!("no pooled embedding for sequence {seq_id}"));
                }
                // SAFETY: a pooled embedding holds n_embd_out floats, which
                // is exactly self.n_embd.
                Ok(unsafe { std::slice::from_raw_parts(v, self.n_embd) }.to_vec())
            })
            .collect()
    }
}

/// Owns a `llama_batch` so every exit path frees it.
struct Batch(ffi::llama_batch);

impl Batch {
    fn new(n_tokens: usize) -> Self {
        // SAFETY: allocates token/pos/n_seq_id/seq_id/logits arrays for
        // n_tokens entries with one seq_id slot each; embd = 0 selects token
        // input.
        Self(unsafe { ffi::llama_batch_init(n_tokens as i32, 0, 1) })
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        // SAFETY: the batch came from llama_batch_init and is freed once.
        unsafe { ffi::llama_batch_free(self.0) };
    }
}

/// Tokenises `text` literally. Specials are not parsed: a completion carrying
/// chat control markers must be embedded as the text it is, not as the
/// control tokens it imitates.
fn tokenise(model: &LlamaModel, text: &str) -> Result<Vec<ffi::llama_token>, String> {
    let cap = EmbeddingContext::N_CTX as usize;
    let len = i32::try_from(text.len()).map_err(|_| "text is too long to tokenise".to_string())?;
    // One slot past the limit, so an over-long text is detected without
    // tokenising the whole of it into an unbounded buffer.
    let mut tokens = vec![0 as ffi::llama_token; cap + 1];
    // SAFETY: model.ptr is valid; the vocab pointer it returns lives as long
    // as the model; text is valid for `len` bytes and tokens is writable for
    // tokens.len() entries.
    let n = unsafe {
        let vocab = ffi::llama_model_get_vocab(model.ptr);
        ffi::llama_tokenize(
            vocab,
            text.as_ptr() as *const c_char,
            len,
            tokens.as_mut_ptr(),
            tokens.len() as i32,
            true,
            false,
        )
    };
    if n < 0 || n as usize > cap {
        return Err(format!("text exceeds the {cap}-token embedding context"));
    }
    if n == 0 {
        return Err("text produced no tokens".to_string());
    }
    tokens.truncate(n as usize);
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_pooling_codes_map_to_their_pooling() {
        assert_eq!(PoolingType::from_ffi(1), Some(PoolingType::Mean));
        assert_eq!(PoolingType::from_ffi(2), Some(PoolingType::Cls));
        assert_eq!(PoolingType::from_ffi(3), Some(PoolingType::Last));
    }

    #[test]
    fn codes_without_a_per_text_vector_map_to_none() {
        for code in [-1, 0, 4, 99] {
            assert_eq!(PoolingType::from_ffi(code), None, "code {code}");
        }
    }

    #[test]
    fn pooling_round_trips_through_its_ffi_code() {
        for p in [PoolingType::Mean, PoolingType::Cls, PoolingType::Last] {
            assert_eq!(PoolingType::from_ffi(i64::from(p.to_ffi())), Some(p));
        }
    }

    /// Cross-checks the metadata read against llama.cpp's own resolution:
    /// a context left at UNSPECIFIED adopts the model's declared pooling.
    #[test]
    fn model_pooling_agrees_with_llama_cpp() {
        let Ok(path) = std::env::var("ARKAVO_TEST_EMBED_MODEL") else {
            eprintln!("skipping: set ARKAVO_TEST_EMBED_MODEL");
            return;
        };
        let model = LlamaModel::from_file(&path).expect("load model");
        // SAFETY: default params by value; model.ptr is valid.
        let ptr = unsafe {
            let mut params = ffi::llama_context_default_params();
            params.embeddings = true;
            params.n_ctx = 256;
            ffi::llama_new_context_with_model(model.ptr, params)
        };
        assert!(!ptr.is_null());
        let ctx = LlamaContext { ptr };
        // SAFETY: ctx.ptr is non-null and owned by `ctx`.
        let resolved = unsafe { ffi::llama_pooling_type(ctx.ptr) };
        assert_eq!(
            model_pooling(&model),
            PoolingType::from_ffi(i64::from(resolved))
        );
    }

    /// A control marker written into the text must stay text: tokenising the
    /// end-of-sequence marker's own spelling must not yield another EOS.
    #[test]
    fn control_marker_spelling_is_not_parsed_as_the_control_token() {
        let Ok(path) = std::env::var("ARKAVO_TEST_EMBED_MODEL") else {
            eprintln!("skipping: set ARKAVO_TEST_EMBED_MODEL");
            return;
        };
        let model = LlamaModel::from_file(&path).expect("load model");
        // SAFETY: model.ptr is valid; the vocab and the token text it returns
        // live as long as the model.
        let (eos, spelling) = unsafe {
            let vocab = ffi::llama_model_get_vocab(model.ptr);
            let eos = ffi::llama_vocab_eos(vocab);
            let text = std::ffi::CStr::from_ptr(ffi::llama_vocab_get_text(vocab, eos));
            (eos, text.to_str().expect("utf-8 marker").to_string())
        };
        // Count against a baseline: a vocab that appends EOS as a special
        // puts it in both, and only a parsed marker adds one more.
        let eos_count = |text: &str| {
            let tokens = tokenise(&model, text).unwrap();
            tokens.iter().filter(|&&t| t == eos).count()
        };
        assert_eq!(
            eos_count(&format!("before {spelling} after")),
            eos_count("before after"),
            "{spelling} was parsed as a control token"
        );
    }
}
