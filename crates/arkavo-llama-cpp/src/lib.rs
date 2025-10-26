// For musl targets, provide stub implementations since llama.cpp doesn't work well with musl
#[cfg(target_env = "musl")]
mod stubs {
    pub fn init_llama_logging() {}
    pub fn set_debug_logging(_enabled: bool) {}
    pub fn test_minimal_init() -> Result<(), String> {
        Err("llama.cpp is not supported on musl targets".to_string())
    }
}

#[cfg(target_env = "musl")]
pub use stubs::*;

// Real implementation for non-musl targets
#[cfg(not(target_env = "musl"))]
pub use arkavo_llama_cpp_sys as ffi;

#[cfg(not(target_env = "musl"))]
use std::ffi::CString;
#[cfg(not(target_env = "musl"))]
use std::os::raw::{c_char, c_void};
#[cfg(not(target_env = "musl"))]
use std::panic;
#[cfg(not(target_env = "musl"))]
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

// Global flag to control llama.cpp logging
#[cfg(not(target_env = "musl"))]
static LLAMA_LOGGING_ENABLED: AtomicBool = AtomicBool::new(false);

// Global flag to track if GPU has failed (avoid retrying)
// 0 = not tried, 1 = GPU works, 2 = GPU failed (use CPU)
#[cfg(not(target_env = "musl"))]
static GPU_STATUS: AtomicU32 = AtomicU32::new(0);

// Custom log callback that filters based on log level and our debug flag
#[cfg(not(target_env = "musl"))]
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
#[cfg(not(target_env = "musl"))]
pub fn init_llama_logging() {
    // Logging disabled by default, can be enabled with set_debug_logging
    LLAMA_LOGGING_ENABLED.store(false, Ordering::Relaxed);

    // Set our custom log callback
    unsafe {
        ffi::llama_log_set(Some(llama_log_callback_filtered), std::ptr::null_mut());
    }
}

/// Enable or disable debug logging for llama.cpp
#[cfg(not(target_env = "musl"))]
pub fn set_debug_logging(enabled: bool) {
    LLAMA_LOGGING_ENABLED.store(enabled, Ordering::Relaxed);
}

#[cfg(not(target_env = "musl"))]
pub struct LlamaModel {
    pub(crate) ptr: *mut ffi::llama_model,
}

// SAFETY: llama.cpp's model objects are thread-safe for read operations
#[cfg(not(target_env = "musl"))]
unsafe impl Send for LlamaModel {}
#[cfg(not(target_env = "musl"))]
unsafe impl Sync for LlamaModel {}

#[cfg(not(target_env = "musl"))]
impl LlamaModel {
    pub fn from_file(path: &str) -> Result<Self, String> {
        // Initialize backend if not already done
        unsafe {
            ffi::llama_backend_init();
        }

        let c_path = CString::new(path).unwrap();

        // Check if GPU has already failed
        let gpu_status = GPU_STATUS.load(Ordering::Relaxed);
        let try_gpu = gpu_status != 2; // Don't try if previously failed

        if try_gpu {
            // First attempt: GPU acceleration
            let mut params = unsafe { ffi::llama_model_default_params() };
            params.n_gpu_layers = 999; // Offload all layers (999 = all)
            params.main_gpu = 0; // Use GPU 0 (primary GPU)

            if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
                eprintln!("Attempting GPU acceleration (offloading all layers)");
            }

            let model = unsafe { ffi::llama_load_model_from_file(c_path.as_ptr(), params) };

            if !model.is_null() {
                GPU_STATUS.store(1, Ordering::Relaxed); // GPU works
                if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
                    eprintln!("✓ GPU model loaded successfully");
                }
                return Ok(Self { ptr: model });
            }

            // GPU failed - mark it and fall back to CPU
            GPU_STATUS.store(2, Ordering::Relaxed);
            eprintln!("⚠ GPU model loading failed, falling back to CPU-only mode");
        } else if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
            eprintln!("Skipping GPU (previous failure), using CPU-only mode");
        }

        // CPU fallback
        let mut cpu_params = unsafe { ffi::llama_model_default_params() };
        cpu_params.n_gpu_layers = 0; // CPU only

        let cpu_model = unsafe { ffi::llama_load_model_from_file(c_path.as_ptr(), cpu_params) };
        if cpu_model.is_null() {
            Err("Failed to load model (CPU attempt failed)".to_string())
        } else {
            eprintln!("✓ CPU-only model loaded successfully");
            Ok(Self { ptr: cpu_model })
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

#[cfg(not(target_env = "musl"))]
impl Drop for LlamaModel {
    fn drop(&mut self) {
        unsafe {
            ffi::llama_free_model(self.ptr);
        }
    }
}

#[cfg(not(target_env = "musl"))]
pub struct LlamaContext {
    pub(crate) ptr: *mut ffi::llama_context,
}

// SAFETY: llama.cpp contexts need to be protected by mutex for thread safety
#[cfg(not(target_env = "musl"))]
unsafe impl Send for LlamaContext {}

#[cfg(not(target_env = "musl"))]
impl LlamaContext {
    pub fn new(model: &LlamaModel) -> Result<Self, String> {
        // Auto-detect CPU cores for optimal thread count
        let num_cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8); // Fallback to 8 if detection fails
        let thread_count = num_cores.min(16) as i32; // Cap at 16 for diminishing returns

        // Detect if running on resource-constrained device (e.g., Raspberry Pi)
        let is_low_power = std::env::var("ARKAVO_RASPBERRY_PI")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or_else(|_| num_cores <= 4);

        // Detect Qualcomm Adreno GPU (via GGML_VK_MAX_BATCH env var or ARM64+Vulkan build)
        let is_adreno = std::env::var("GGML_VK_MAX_BATCH").is_ok()
            || (cfg!(target_arch = "aarch64") && cfg!(feature = "llama-cpp"));

        // Try to create context, catch Vulkan crashes
        let gpu_status = GPU_STATUS.load(Ordering::Relaxed);

        // Try GPU if it hasn't failed before
        if gpu_status != 2 {
            let mut gpu_params = unsafe { ffi::llama_context_default_params() };

            if is_adreno {
                gpu_params.n_ctx = 2048;
                gpu_params.n_batch = 16;
                gpu_params.n_ubatch = 16;
            } else if is_low_power {
                gpu_params.n_ctx = 2048;
                gpu_params.n_batch = 512;
                gpu_params.n_ubatch = 256;
            } else {
                gpu_params.n_ctx = 32768;
                gpu_params.n_batch = 2048;
                gpu_params.n_ubatch = 512;
            }
            gpu_params.n_seq_max = 1;
            gpu_params.n_threads = thread_count;
            gpu_params.n_threads_batch = thread_count;
            gpu_params.offload_kqv = true;
            gpu_params.flash_attn_type = ffi::llama_flash_attn_type_LLAMA_FLASH_ATTN_TYPE_AUTO;

            // Try with panic catching (Vulkan may abort)
            let gpu_result = panic::catch_unwind(panic::AssertUnwindSafe(|| unsafe {
                ffi::llama_new_context_with_model(model.ptr, gpu_params)
            }));

            match gpu_result {
                Ok(context) if !context.is_null() => {
                    GPU_STATUS.store(1, Ordering::Relaxed);
                    if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
                        eprintln!("✓ GPU context created successfully");
                    }
                    return Ok(Self { ptr: context });
                }
                Ok(_) => {
                    GPU_STATUS.store(2, Ordering::Relaxed);
                    eprintln!("⚠ GPU context creation failed (returned null), falling back to CPU");
                }
                Err(_) => {
                    GPU_STATUS.store(2, Ordering::Relaxed);
                    eprintln!(
                        "⚠ GPU context creation crashed (Vulkan driver error), falling back to CPU"
                    );
                }
            }
        }

        // CPU fallback
        let mut cpu_params = unsafe { ffi::llama_context_default_params() };

        if is_adreno || is_low_power {
            cpu_params.n_ctx = 2048;
            cpu_params.n_batch = 512;
            cpu_params.n_ubatch = 256;
        } else {
            cpu_params.n_ctx = 32768;
            cpu_params.n_batch = 2048;
            cpu_params.n_ubatch = 512;
        }
        cpu_params.n_seq_max = 1;
        cpu_params.n_threads = thread_count;
        cpu_params.n_threads_batch = thread_count;
        cpu_params.offload_kqv = false; // CPU only
        cpu_params.flash_attn_type = ffi::llama_flash_attn_type_LLAMA_FLASH_ATTN_TYPE_DISABLED;

        if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
            eprintln!(
                "Creating CPU-only context: n_ctx={}, n_batch={}, threads={}",
                cpu_params.n_ctx, cpu_params.n_batch, thread_count
            );
        }

        let context = unsafe { ffi::llama_new_context_with_model(model.ptr, cpu_params) };
        if context.is_null() {
            Err("Failed to create context (CPU attempt failed)".to_string())
        } else {
            eprintln!("✓ CPU-only context created successfully");
            Ok(Self { ptr: context })
        }
    }

    pub fn get_logits_ith(&self, i: i32) -> *mut f32 {
        unsafe { ffi::llama_get_logits_ith(self.ptr, i) }
    }

    /// Clear the KV cache - no-op, managed automatically
    pub fn clear_kv_cache(&self) {
        if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
            eprintln!("[DEBUG] KV cache managed automatically");
        }
    }

    /// Remove a specific sequence from the KV cache - API removed in newer llama.cpp
    pub fn remove_sequence(&self, _seq_id: i32, _pos_start: i32, _pos_end: i32) -> bool {
        if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
            eprintln!("[DEBUG] Remove sequence skipped (API removed)");
        }
        true
    }
}

#[cfg(not(target_env = "musl"))]
impl Drop for LlamaContext {
    fn drop(&mut self) {
        unsafe {
            ffi::llama_free(self.ptr);
        }
    }
}

#[cfg(not(target_env = "musl"))]
pub fn apply_chat_template(
    messages: &[ffi::llama_chat_message],
    add_assistant: bool,
) -> Result<Vec<u8>, String> {
    // Gemma-3 chat template - simple format for small models
    let gemma3_template = "{% for message in messages %}{% if message['role'] == 'user' %}{{'<start_of_turn>user\n' + message['content'] + '<end_of_turn>\n'}}{% elif message['role'] == 'assistant' %}{{'<start_of_turn>model\n' + message['content'] + '<end_of_turn>\n'}}{% endif %}{% endfor %}{% if add_generation_prompt %}<start_of_turn>model\n{% endif %}";

    let template_cstring = CString::new(gemma3_template)
        .map_err(|e| format!("Failed to create template CString: {}", e))?;

    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let wrote = unsafe {
            ffi::llama_chat_apply_template(
                template_cstring.as_ptr(),
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

#[cfg(not(target_env = "musl"))]
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

#[cfg(not(target_env = "musl"))]
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

#[cfg(not(target_env = "musl"))]
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

#[cfg(not(target_env = "musl"))]
pub fn batch_get_one(tokens: &[ffi::llama_token]) -> ffi::llama_batch {
    unsafe {
        ffi::llama_batch_get_one(
            tokens.as_ptr() as *mut ffi::llama_token,
            tokens.len() as i32,
        )
    }
}

#[cfg(not(target_env = "musl"))]
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

#[cfg(not(target_env = "musl"))]
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
#[cfg(not(target_env = "musl"))]
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
#[cfg(not(target_env = "musl"))]
pub fn batch_free(batch: &mut ffi::llama_batch) {
    unsafe {
        ffi::llama_batch_free(*batch);
    }
}

#[cfg(not(target_env = "musl"))]
pub fn decode_batch(ctx: &LlamaContext, batch: ffi::llama_batch) -> Result<(), String> {
    let result = unsafe { ffi::llama_decode(ctx.ptr, batch) };
    if result != 0 {
        Err(format!("llama_decode failed with code: {}", result))
    } else {
        Ok(())
    }
}

#[cfg(not(target_env = "musl"))]
pub fn get_logits_ith(ctx: &LlamaContext, i: i32) -> *mut f32 {
    unsafe { ffi::llama_get_logits_ith(ctx.ptr, i) }
}

#[cfg(not(target_env = "musl"))]
pub struct LlamaSampler {
    ptr: *mut ffi::llama_sampler,
}

#[cfg(not(target_env = "musl"))]
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

#[cfg(not(target_env = "musl"))]
unsafe impl Send for LlamaSampler {}

#[cfg(not(target_env = "musl"))]
impl Drop for LlamaSampler {
    fn drop(&mut self) {
        unsafe {
            ffi::llama_sampler_free(self.ptr);
        }
    }
}

#[cfg(not(target_env = "musl"))]
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
#[cfg(not(target_env = "musl"))]
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
