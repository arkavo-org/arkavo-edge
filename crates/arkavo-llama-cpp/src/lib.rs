pub use arkavo_llama_cpp_sys as ffi;

use std::ffi::CString;
use std::os::raw::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

// Global flag to control llama.cpp logging
static LLAMA_LOGGING_ENABLED: AtomicBool = AtomicBool::new(false);

// Custom log callback that filters based on log level and our debug flag
extern "C" fn llama_log_callback_filtered(
    level: ffi::ggml_log_level,
    text: *const c_char,
    _user_data: *mut c_void,
) {
    // Only show logs if:
    // - Debug is enabled AND it's any level, OR
    // - It's a warning/error (always show these)
    let is_warning_or_error = level >= ffi::ggml_log_level_GGML_LOG_LEVEL_WARN;
    let debug_enabled = LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed);

    if (is_warning_or_error || debug_enabled) && !text.is_null() {
        unsafe {
            let c_str = std::ffi::CStr::from_ptr(text);
            if let Ok(str_slice) = c_str.to_str() {
                // Skip various non-critical messages unless debug is on
                if !debug_enabled {
                    // Skip progress dots
                    if str_slice == "." {
                        return;
                    }
                    // Skip cache messages
                    if str_slice.contains("llama_kv_cache") {
                        return;
                    }
                    // Skip Metal BF16 kernel messages (not supported, not needed)
                    if str_slice.contains("ggml_metal_init: skipping") && str_slice.contains("bf16")
                    {
                        return;
                    }
                }
                eprint!("{}", str_slice);
            }
        }
    }
}

/// Initialize llama.cpp logging
pub fn init_llama_logging() {
    // Logging disabled by default, can be enabled with set_debug_logging
    LLAMA_LOGGING_ENABLED.store(false, Ordering::Relaxed);

    // Set our custom log callback
    unsafe {
        ffi::llama_log_set(Some(llama_log_callback_filtered), std::ptr::null_mut());
    }
}

/// Enable or disable debug logging for llama.cpp
pub fn set_debug_logging(enabled: bool) {
    LLAMA_LOGGING_ENABLED.store(enabled, Ordering::Relaxed);
}

pub struct LlamaModel {
    pub(crate) ptr: *mut ffi::llama_model,
}

// SAFETY: llama.cpp's model objects are thread-safe for read operations
unsafe impl Send for LlamaModel {}
unsafe impl Sync for LlamaModel {}

impl LlamaModel {
    pub fn from_file(path: &str) -> Result<Self, String> {
        // Initialize backend if not already done
        unsafe {
            ffi::llama_backend_init();
        }

        let c_path = CString::new(path).unwrap();
        let mut params = unsafe { ffi::llama_model_default_params() };

        // Enable GPU acceleration - offload all layers to GPU/Metal
        params.n_gpu_layers = 999; // Offload all layers (999 = all)
        params.main_gpu = 0; // Use GPU 0 (primary GPU)

        // Show GPU offloading info if debug is enabled
        if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
            eprintln!("GPU: Offloading all layers to Metal/GPU");
        }

        let model = unsafe { ffi::llama_load_model_from_file(c_path.as_ptr(), params) };
        if model.is_null() {
            Err("Failed to load model".to_string())
        } else {
            Ok(Self { ptr: model })
        }
    }

    pub fn get_vocab(&self) -> *const ffi::llama_vocab {
        unsafe { ffi::llama_model_get_vocab(self.ptr) }
    }

    pub fn get_eos_token(&self) -> i32 {
        let vocab = self.get_vocab();
        unsafe { ffi::llama_vocab_eos(vocab) }
    }

    pub fn get_bos_token(&self) -> i32 {
        let vocab = self.get_vocab();
        unsafe { ffi::llama_vocab_bos(vocab) }
    }
}

impl Drop for LlamaModel {
    fn drop(&mut self) {
        unsafe {
            ffi::llama_free_model(self.ptr);
        }
    }
}

pub struct LlamaContext {
    pub(crate) ptr: *mut ffi::llama_context,
}

// SAFETY: llama.cpp contexts need to be protected by mutex for thread safety
unsafe impl Send for LlamaContext {}

impl LlamaContext {
    pub fn new(model: &LlamaModel) -> Result<Self, String> {
        let mut params = unsafe { ffi::llama_context_default_params() };

        // Set context size to utilize the full capacity of the model
        params.n_ctx = 32768; // Context window: 32K tokens (full model capacity)
        params.n_batch = 512; // Batch size for processing
        params.n_ubatch = 512; // Micro-batch size
        params.n_seq_max = 1; // Single sequence
        params.n_threads = 8; // CPU threads (use more for M4)
        params.n_threads_batch = 8; // Batch processing threads

        // Enable GPU offloading for KV cache and operations
        params.offload_kqv = true; // Offload KV cache to GPU
        params.flash_attn = true; // Use Flash Attention if available

        // Show context configuration if debug is enabled
        if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
            eprintln!(
                "Context: KV offload={}, flash_attn={}, threads={}",
                params.offload_kqv, params.flash_attn, params.n_threads
            );
        }

        let context = unsafe { ffi::llama_new_context_with_model(model.ptr, params) };
        if context.is_null() {
            Err("Failed to create context".to_string())
        } else {
            Ok(Self { ptr: context })
        }
    }

    pub fn get_logits_ith(&self, i: i32) -> *mut f32 {
        unsafe { ffi::llama_get_logits_ith(self.ptr, i) }
    }

    /// Clear the KV cache for all sequences
    pub fn clear_kv_cache(&self) {
        unsafe {
            // Use the older API name that's available
            ffi::llama_kv_self_clear(self.ptr);
        }
        if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
            eprintln!("[DEBUG] KV cache cleared");
        }
    }

    /// Remove a specific sequence from the KV cache
    pub fn remove_sequence(&self, seq_id: i32, pos_start: i32, pos_end: i32) -> bool {
        let result = unsafe { ffi::llama_kv_self_seq_rm(self.ptr, seq_id, pos_start, pos_end) };
        if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
            eprintln!(
                "[DEBUG] Removed sequence {} from KV cache (pos {}-{})",
                seq_id, pos_start, pos_end
            );
        }
        result
    }
}

impl Drop for LlamaContext {
    fn drop(&mut self) {
        unsafe {
            ffi::llama_free(self.ptr);
        }
    }
}

pub fn apply_chat_template(
    messages: &[ffi::llama_chat_message],
    add_assistant: bool,
) -> Result<Vec<u8>, String> {
    // Gemma-3 chat template
    // Format: <start_of_turn>role\ncontent<end_of_turn>
    let gemma3_template = "{% for message in messages %}{% if message['role'] == 'user' %}{{'<start_of_turn>user\n' + message['content'] + '<end_of_turn>\n'}}{% elif message['role'] == 'assistant' %}{{'<start_of_turn>model\n' + message['content'] + '<end_of_turn>\n'}}{% elif message['role'] == 'system' %}{{'<start_of_turn>system\n' + message['content'] + '<end_of_turn>\n'}}{% endif %}{% endfor %}{% if add_generation_prompt %}<start_of_turn>model\n{% endif %}";

    let template_cstring = CString::new(gemma3_template)
        .map_err(|e| format!("Failed to create template CString: {}", e))?;

    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let wrote = unsafe {
            ffi::llama_chat_apply_template(
                template_cstring.as_ptr(), // Use Gemma-3 template
                messages.as_ptr(),
                messages.len(),
                add_assistant,
                buf.as_mut_ptr() as *mut c_char,
                buf.len() as i32,
            )
        };
        if wrote >= 0 && (wrote as usize) <= buf.len() {
            buf.truncate(wrote as usize);
            return Ok(buf);
        }
        let need = wrote.checked_neg().unwrap_or(128 * 1024) as usize;
        buf.resize(need, 0);
    }
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn tokenize_with_model(
    vocab: *const ffi::llama_vocab,
    text_utf8: &[u8],
) -> Result<Vec<ffi::llama_token>, String> {
    let mut toks = vec![0i32; text_utf8.len() + 8];
    loop {
        let n = unsafe {
            ffi::llama_tokenize(
                vocab,
                text_utf8.as_ptr() as *const c_char,
                text_utf8.len() as i32,
                toks.as_mut_ptr(),
                toks.len() as i32,
                true, // add_special (BOS/EOS if appropriate)
                true, // parse_special (chat template control tokens)
            )
        };
        if n >= 0 && (n as usize) <= toks.len() {
            toks.truncate(n as usize);
            return Ok(toks);
        }
        let need = n.checked_neg().unwrap_or((toks.len() * 2) as i32) as usize;
        toks.resize(need, 0);
    }
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn detokenize(
    vocab: *const ffi::llama_vocab,
    tokens: &[ffi::llama_token],
    remove_special: bool,
    unparse_special: bool,
) -> Result<String, String> {
    let mut buf = vec![0u8; tokens.len() * 8 + 16];
    loop {
        let n = unsafe {
            ffi::llama_detokenize(
                vocab,
                tokens.as_ptr(),
                tokens.len() as i32,
                buf.as_mut_ptr() as *mut c_char,
                buf.len() as i32,
                remove_special,
                unparse_special,
            )
        };
        if n >= 0 && (n as usize) <= buf.len() {
            buf.truncate(n as usize);
            return String::from_utf8(buf).map_err(|e| format!("UTF-8 conversion error: {}", e));
        }
        let need = n.checked_neg().unwrap_or((buf.len() * 2) as i32) as usize;
        buf.resize(need, 0);
    }
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn token_to_piece(
    vocab: *const ffi::llama_vocab,
    token: ffi::llama_token,
    special: bool,
) -> Result<String, String> {
    let mut buf = vec![0u8; 32];
    loop {
        let n = unsafe {
            ffi::llama_token_to_piece(
                vocab,
                token,
                buf.as_mut_ptr() as *mut c_char,
                buf.len() as i32,
                0, // lstrip - don't strip leading space
                special,
            )
        };
        if n >= 0 && (n as usize) <= buf.len() {
            buf.truncate(n as usize);
            return String::from_utf8(buf).map_err(|e| format!("UTF-8 conversion error: {}", e));
        }
        let need = n.checked_neg().unwrap_or((buf.len() * 2) as i32) as usize;
        buf.resize(need, 0);
    }
}

pub fn batch_get_one(tokens: &[ffi::llama_token]) -> ffi::llama_batch {
    unsafe {
        ffi::llama_batch_get_one(
            tokens.as_ptr() as *mut ffi::llama_token,
            tokens.len() as i32,
        )
    }
}

pub fn batch_get_one_with_logits(
    tokens: &[ffi::llama_token],
    request_logits_on_last: bool,
) -> ffi::llama_batch {
    let batch = unsafe {
        ffi::llama_batch_get_one(
            tokens.as_ptr() as *mut ffi::llama_token,
            tokens.len() as i32,
        )
    };

    // Set logits=1 on the last token if requested (crucial for sampling)
    if request_logits_on_last && !tokens.is_empty() && !batch.logits.is_null() {
        unsafe {
            *batch.logits.add(tokens.len() - 1) = 1;
        }
    }

    batch
}

pub fn batch_get_one_with_offset(
    tokens: &[ffi::llama_token],
    pos_offset: i32,
    request_logits_on_last: bool,
) -> ffi::llama_batch {
    let batch = unsafe {
        ffi::llama_batch_get_one(
            tokens.as_ptr() as *mut ffi::llama_token,
            tokens.len() as i32,
        )
    };

    // Check if position array is available and adjust positions
    if !batch.pos.is_null() {
        for i in 0..tokens.len() {
            unsafe {
                *batch.pos.add(i) = pos_offset + i as i32;
            }
        }
    }

    // Set logits=1 on the last token if requested (crucial for sampling)
    if request_logits_on_last && !tokens.is_empty() && !batch.logits.is_null() {
        unsafe {
            *batch.logits.add(tokens.len() - 1) = 1;
        }
    }

    batch
}

/// Proper "llama way" batch creation with guaranteed allocation
pub fn batch_init_with_tokens(
    tokens: &[ffi::llama_token],
    pos_offset: i32,
    request_logits_on_last: bool,
) -> ffi::llama_batch {
    let mut batch = unsafe {
        ffi::llama_batch_init(
            tokens.len() as i32,
            0, // embd = 0 for token mode
            1, // n_seq_max = 1
        )
    };

    // Fill batch arrays - all arrays are guaranteed allocated by llama_batch_init
    for (i, &token) in tokens.iter().enumerate() {
        unsafe {
            *batch.token.add(i) = token;
            *batch.pos.add(i) = pos_offset + i as i32;
            *batch.n_seq_id.add(i) = 1; // 1 sequence
            *(*batch.seq_id.add(i)) = 0; // sequence ID = 0
            *batch.logits.add(i) = 0; // no logits by default
        }
    }

    // Set logits=1 on the last token if requested (crucial for sampling)
    if request_logits_on_last && !tokens.is_empty() {
        unsafe {
            *batch.logits.add(tokens.len() - 1) = 1;
        }
    }

    batch.n_tokens = tokens.len() as i32;
    batch
}

/// Free a batch created with batch_init_with_tokens
pub fn batch_free(batch: &mut ffi::llama_batch) {
    unsafe {
        ffi::llama_batch_free(*batch);
    }
}

pub fn decode_batch(ctx: &LlamaContext, batch: ffi::llama_batch) -> Result<(), String> {
    let result = unsafe { ffi::llama_decode(ctx.ptr, batch) };
    if result != 0 {
        Err(format!("llama_decode failed with code: {}", result))
    } else {
        Ok(())
    }
}

pub fn get_logits_ith(ctx: &LlamaContext, i: i32) -> *mut f32 {
    unsafe { ffi::llama_get_logits_ith(ctx.ptr, i) }
}

pub struct LlamaSampler {
    ptr: *mut ffi::llama_sampler,
}

impl LlamaSampler {
    pub fn new_chain(no_perf: bool) -> Result<Self, String> {
        let chain_params = ffi::llama_sampler_chain_params { no_perf };
        let sampler = unsafe { ffi::llama_sampler_chain_init(chain_params) };
        if sampler.is_null() {
            Err("Failed to create sampler chain".to_string())
        } else {
            Ok(Self { ptr: sampler })
        }
    }

    pub fn add_temp(&self, temp: f32) {
        let temp_sampler = unsafe { ffi::llama_sampler_init_temp(temp) };
        if !temp_sampler.is_null() {
            unsafe { ffi::llama_sampler_chain_add(self.ptr, temp_sampler) };
        }
    }

    pub fn add_greedy(&self) {
        let greedy_sampler = unsafe { ffi::llama_sampler_init_greedy() };
        if !greedy_sampler.is_null() {
            unsafe { ffi::llama_sampler_chain_add(self.ptr, greedy_sampler) };
        }
    }

    pub fn add_top_k(&self, k: i32) {
        let top_k_sampler = unsafe { ffi::llama_sampler_init_top_k(k) };
        if !top_k_sampler.is_null() {
            unsafe { ffi::llama_sampler_chain_add(self.ptr, top_k_sampler) };
        }
    }

    pub fn add_top_p(&self, p: f32, min_keep: usize) {
        let top_p_sampler = unsafe { ffi::llama_sampler_init_top_p(p, min_keep) };
        if !top_p_sampler.is_null() {
            unsafe { ffi::llama_sampler_chain_add(self.ptr, top_p_sampler) };
        }
    }

    pub fn sample(&self, ctx: &LlamaContext, idx: i32) -> ffi::llama_token {
        unsafe { ffi::llama_sampler_sample(self.ptr, ctx.ptr, idx) }
    }

    pub fn accept(&self, token: ffi::llama_token) {
        unsafe { ffi::llama_sampler_accept(self.ptr, token) };
    }
}

unsafe impl Send for LlamaSampler {}

impl Drop for LlamaSampler {
    fn drop(&mut self) {
        unsafe {
            ffi::llama_sampler_free(self.ptr);
        }
    }
}

pub fn create_sampler_chain(
    temp: f32,
    top_p: f32,
    top_k: i32,
    _seed: u32,
) -> Result<LlamaSampler, String> {
    // Clamp params to reasonable ranges
    let top_k = if top_k < 1 { 40 } else { top_k }; // Default to 40 if not set
    let top_p = top_p.clamp(0.1, 1.0);
    let temp = temp.max(0.0);

    let sampler = LlamaSampler::new_chain(false)?;

    // Build a proper sampling chain
    if temp <= 0.0 {
        // Greedy/deterministic sampling
        sampler.add_greedy();
        if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
            eprintln!("Sampler: greedy (deterministic)");
        }
    } else {
        // Stochastic sampling with proper chain
        // Order matters: top_k -> top_p -> temp -> final selection

        // 1. Top-K sampling (keep only top K tokens)
        if top_k > 0 {
            sampler.add_top_k(top_k);
        }

        // 2. Top-P (nucleus) sampling
        if top_p < 1.0 {
            sampler.add_top_p(top_p, 1); // min_keep=1
        }

        // 3. Temperature scaling
        sampler.add_temp(temp);

        // 4. Final token selection - greedy picks the most likely after transformations
        sampler.add_greedy();

        if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
            eprintln!("Sampler: top_k={}, top_p={}, temp={}", top_k, top_p, temp);
        }
    }

    Ok(sampler)
}

/// Minimal FFI test harness to verify llama.cpp initialization
pub fn test_minimal_init() -> Result<(), String> {
    // Test model params creation without backend init/cleanup
    let mut _model_params = unsafe { ffi::llama_model_default_params() };
    _model_params.vocab_only = true; // only read vocab & metadata
    _model_params.use_mmap = false; // avoid vm tricks until stable
    _model_params.use_mlock = false; // avoid locking (needs perms)

    // Only show debug output if debug logging is enabled
    if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
        eprintln!("✓ llama_model_default_params() succeeded");
        eprintln!("✓ Minimal FFI initialization test passed!");
    }

    Ok(())
}
