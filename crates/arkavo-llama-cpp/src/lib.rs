/// GPU acceleration status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuStatus {
    /// GPU not yet tested (no model loaded yet)
    Unknown,
    /// GPU acceleration available and working
    Available,
    /// GPU unavailable, running in CPU-only mode
    Unavailable,
}

// For musl targets, provide stub implementations since llama.cpp doesn't work well with musl
#[cfg(target_env = "musl")]
mod stubs {
    use super::GpuStatus;

    pub fn init_llama_logging() {}
    pub fn set_debug_logging(_enabled: bool) {}
    pub fn test_minimal_init() -> Result<(), String> {
        Err("llama.cpp is not supported on musl targets".to_string())
    }
    pub fn gpu_status() -> GpuStatus {
        GpuStatus::Unavailable
    }
}

#[cfg(target_env = "musl")]
pub use stubs::*;

// Multimodal support module
#[cfg(not(target_env = "musl"))]
pub mod multimodal;

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

/// Model format for chat template selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelFormat {
    /// Gemma-3 format: <start_of_turn>user\n...<end_of_turn>
    Gemma3,
    /// Mistral V3 format: [SYSTEM_PROMPT]...[/SYSTEM_PROMPT] [INST]...[/INST]
    MistralV3,
    /// Qwen3 format (ChatML): <|im_start|>user\n...<|im_end|>
    #[default]
    Qwen3,
    /// GLM-4 format (ChatML variant): [gMASK]<sop><|user|>\n...<|assistant|>\n
    GLM4,
}

/// Detect model format from model name or path
pub fn detect_model_format(model_name: &str) -> ModelFormat {
    let name_lower = model_name.to_lowercase();
    if name_lower.contains("glm-4") || name_lower.contains("glm4") {
        ModelFormat::GLM4
    } else if name_lower.contains("qwen") {
        ModelFormat::Qwen3
    } else if name_lower.contains("mistral") || name_lower.contains("ministral") {
        ModelFormat::MistralV3
    } else if name_lower.contains("gemma") {
        ModelFormat::Gemma3
    } else {
        // Default to Qwen3 for unknown models (TØRG-compatible)
        ModelFormat::Qwen3
    }
}

// Global flag to track if GPU has failed (avoid retrying)
// 0 = not tried, 1 = GPU works, 2 = GPU failed (use CPU)
#[cfg(not(target_env = "musl"))]
static GPU_STATUS: AtomicU32 = AtomicU32::new(0);

/// Check if GPU acceleration is available for local inference
///
/// Returns `Unknown` if no model has been loaded yet (GPU status not tested).
/// Returns `Available` if GPU acceleration is working.
/// Returns `Unavailable` if GPU failed and running in CPU-only mode.
#[cfg(not(target_env = "musl"))]
pub fn gpu_status() -> GpuStatus {
    match GPU_STATUS.load(Ordering::Relaxed) {
        0 => GpuStatus::Unknown,
        1 => GpuStatus::Available,
        _ => GpuStatus::Unavailable,
    }
}

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
        // SAFETY: CString is valid null-terminated UTF-8; pointer is valid for the duration of the call
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
                    // Skip context size info messages (we handle this ourselves)
                    if str_slice.contains("n_ctx_per_seq") || str_slice.contains("n_ctx_train") {
                        return;
                    }
                    // Skip Metal tensor API message (informational for pre-M5/A19 devices)
                    if str_slice.contains("tensor API disabled for pre-M") {
                        return;
                    }
                    // Skip tokenizer config warnings for models with non-standard special token handling
                    if str_slice.contains("is not in special_eog_ids") {
                        return;
                    }
                    // Skip llama_context informational messages (yarn_attn_factor, etc.)
                    if str_slice.contains("llama_context") {
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

    // SAFETY: CString is valid null-terminated UTF-8; pointer is valid for the duration of the call
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
    path: String,
}

// SAFETY: llama.cpp's model objects are thread-safe for read operations
#[cfg(not(target_env = "musl"))]
unsafe impl Send for LlamaModel {}
#[cfg(not(target_env = "musl"))]
unsafe impl Sync for LlamaModel {}

#[cfg(not(target_env = "musl"))]
impl LlamaModel {
    pub fn from_file(path: &str) -> Result<Self, String> {
        Self::from_file_with_options(path, true, false)
    }

    /// Load model with explicit options
    /// - `use_direct_io`: Use direct I/O for faster loading (bypasses OS cache)
    /// - `use_mmap`: Use memory-mapped I/O (default: true, ignored if use_direct_io is true)
    pub fn from_file_with_options(
        path: &str,
        use_mmap: bool,
        use_direct_io: bool,
    ) -> Result<Self, String> {
        // SAFETY: Null return is checked immediately after this call
        unsafe {
            ffi::llama_backend_init();
        }

        let c_path = CString::new(path).unwrap();

        // Check if GPU has already failed
        let gpu_status = GPU_STATUS.load(Ordering::Relaxed);
        let try_gpu = gpu_status != 2; // Don't try if previously failed

        if try_gpu {
            // First attempt: GPU acceleration
            // SAFETY: Null return is checked immediately after this call
            let mut params = unsafe { ffi::llama_model_default_params() };
            params.n_gpu_layers = -1; // Offload all layers (negative = all in llama.cpp b7785+)
            params.main_gpu = 0; // Use GPU 0 (primary GPU)
            params.use_mmap = use_mmap;

            if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
                eprintln!(
                    "Attempting GPU acceleration (all layers, mmap={}, direct_io={})",
                    use_mmap, use_direct_io
                );
            }

            // SAFETY: CString is valid null-terminated UTF-8; pointer is valid for the duration of the call
            let model = unsafe { ffi::llama_load_model_from_file(c_path.as_ptr(), params) };

            if !model.is_null() {
                GPU_STATUS.store(1, Ordering::Relaxed); // GPU works
                if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
                    eprintln!("✓ GPU model loaded successfully");
                }
                return Ok(Self {
                    ptr: model,
                    path: path.to_string(),
                });
            }

            // GPU failed - mark it and fall back to CPU
            GPU_STATUS.store(2, Ordering::Relaxed);
            eprintln!("⚠ GPU model loading failed, falling back to CPU-only mode");
        } else if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
            eprintln!("Skipping GPU (previous failure), using CPU-only mode");
        }

        // CPU fallback
        // SAFETY: Null return is checked immediately after this call
        let mut cpu_params = unsafe { ffi::llama_model_default_params() };
        cpu_params.n_gpu_layers = 0; // CPU only
        cpu_params.use_mmap = use_mmap;

        // SAFETY: CString is valid null-terminated UTF-8; pointer is valid for the duration of the call
        let cpu_model = unsafe { ffi::llama_load_model_from_file(c_path.as_ptr(), cpu_params) };
        if cpu_model.is_null() {
            Err("Failed to load model (CPU attempt failed)".to_string())
        } else {
            eprintln!("✓ CPU-only model loaded successfully");
            Ok(Self {
                ptr: cpu_model,
                path: path.to_string(),
            })
        }
    }

    pub fn get_vocab(&self) -> *const ffi::llama_vocab {
        // SAFETY: Null return is checked immediately after this call
        unsafe { ffi::llama_model_get_vocab(self.ptr) }
    }

    pub fn get_eos_token(&self) -> i32 {
        let vocab = self.get_vocab();
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        unsafe { ffi::llama_vocab_eos(vocab) }
    }

    pub fn get_bos_token(&self) -> i32 {
        let vocab = self.get_vocab();
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        unsafe { ffi::llama_vocab_bos(vocab) }
    }

    pub fn get_trained_context_size(&self) -> u32 {
        if self.ptr.is_null() {
            eprintln!("⚠ Model pointer is null, returning default context size");
            return 32768;
        }
        // SAFETY: Null return is checked immediately after this call
        let ctx = unsafe { ffi::llama_n_ctx_train(self.ptr) };
        if ctx <= 0 {
            eprintln!("⚠ Invalid trained context size: {}, returning default", ctx);
            return 32768;
        }
        ctx as u32
    }

    pub fn model_name(&self) -> &str {
        self.path.split('/').next_back().unwrap_or(&self.path)
    }

    /// Get the output embedding size (new in b7785)
    pub fn n_embd_out(&self) -> i32 {
        // Upstream removed/changed the output-embedding accessor in newer llama.cpp.
        // For decoder-only models we use in-embedding size as the stable fallback.
        // SAFETY: Null return is checked immediately after this call
        unsafe { ffi::llama_model_n_embd_inp(self.ptr) }
    }
}

/// Result of llama_params_fit operation
#[cfg(not(target_env = "musl"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamsFitStatus {
    /// Found allocations that are projected to fit
    Success,
    /// Could not find allocations that are projected to fit
    Failure,
    /// A hard error occurred (e.g., model not found)
    Error,
}

/// Auto-fit model and context parameters to available device memory.
/// Returns optimal n_gpu_layers and n_ctx values that should fit in VRAM.
///
/// This is useful for automatically configuring models on systems with limited GPU memory.
#[cfg(not(target_env = "musl"))]
pub fn params_fit(model_path: &str, n_ctx_min: u32) -> Result<(i32, u32), String> {
    // llama_params_fit is not exposed in current bindings; use a conservative fallback.
    // We return CPU-only (0 GPU layers) with at least the model's trained context.
    let model = LlamaModel::from_file(model_path)?;
    Ok((0, model.get_trained_context_size().max(n_ctx_min)))
}

#[cfg(not(target_env = "musl"))]
impl Drop for LlamaModel {
    fn drop(&mut self) {
        // SAFETY: Pointer was allocated by llama.cpp FFI and is guaranteed non-null after construction
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

        // Detect Qualcomm Adreno GPU (Android devices only, not Apple Silicon)
        let is_apple_silicon = cfg!(all(target_arch = "aarch64", target_os = "macos"));
        let is_adreno = std::env::var("GGML_VK_MAX_BATCH").is_ok()
            || (cfg!(target_arch = "aarch64") && !is_apple_silicon);

        // Allow manual context size override
        let manual_ctx = std::env::var("ARKAVO_N_CTX")
            .ok()
            .and_then(|v| v.parse::<u32>().ok());

        // Try to create context, catch Vulkan crashes
        let gpu_status = GPU_STATUS.load(Ordering::Relaxed);

        // Get trained context size from model metadata
        let trained_ctx = model.get_trained_context_size();
        let model_name = model.model_name();

        // Scale context based on trained size to prevent memory exhaustion
        // KV cache memory usage: ~460KB per token for typical small models
        // On 16GB systems, safe limit is ~16K tokens (~7.5GB KV cache + model + system)
        let safe_ctx = if !(512..=1048576).contains(&trained_ctx) {
            eprintln!(
                "⚠ Model '{}' reported unusual trained context size: {}, using 8192",
                model_name, trained_ctx
            );
            8192
        } else if trained_ctx <= 8192 {
            // Small models: use full trained context
            trained_ctx
        } else if trained_ctx <= 32768 {
            // Medium models: use 50% to save memory
            trained_ctx / 2
        } else {
            // Large models: use 25%, capped at 16K to prevent OOM
            (trained_ctx / 4).min(16384)
        };

        if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
            eprintln!(
                "Model '{}': trained_ctx={}, using n_ctx={}",
                model_name, trained_ctx, safe_ctx
            );
        }

        // Try GPU if it hasn't failed before
        if gpu_status != 2 {
            // SAFETY: Null return is checked immediately after this call
            let mut gpu_params = unsafe { ffi::llama_context_default_params() };

            if let Some(ctx) = manual_ctx {
                // Use manual override
                gpu_params.n_ctx = ctx;
                gpu_params.n_batch = (ctx / 16).clamp(16, 2048);
                gpu_params.n_ubatch = (ctx / 32).clamp(16, 512);
            } else if is_adreno {
                gpu_params.n_ctx = 2048;
                gpu_params.n_batch = 16;
                gpu_params.n_ubatch = 16;
            } else if is_low_power {
                gpu_params.n_ctx = 2048;
                gpu_params.n_batch = 512;
                gpu_params.n_ubatch = 256;
            } else {
                // Use validated model context size
                gpu_params.n_ctx = safe_ctx;
                gpu_params.n_batch = 2048;
                gpu_params.n_ubatch = 512;
            }
            gpu_params.n_seq_max = 1;
            gpu_params.n_threads = thread_count;
            gpu_params.n_threads_batch = thread_count;
            gpu_params.offload_kqv = true;
            gpu_params.flash_attn_type = ffi::llama_flash_attn_type_LLAMA_FLASH_ATTN_TYPE_AUTO;

            // Try with panic catching (Vulkan may abort)
            // SAFETY: Null return is checked immediately after this call
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
        // SAFETY: Null return is checked immediately after this call
        let mut cpu_params = unsafe { ffi::llama_context_default_params() };

        if let Some(ctx) = manual_ctx {
            // Use manual override
            cpu_params.n_ctx = ctx;
            cpu_params.n_batch = (ctx / 16).clamp(16, 2048);
            cpu_params.n_ubatch = (ctx / 32).clamp(16, 512);
        } else if is_adreno || is_low_power {
            cpu_params.n_ctx = 2048;
            cpu_params.n_batch = 512;
            cpu_params.n_ubatch = 256;
        } else {
            // Use validated model context size
            cpu_params.n_ctx = safe_ctx;
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

        // SAFETY: Null return is checked immediately after this call
        let context = unsafe { ffi::llama_new_context_with_model(model.ptr, cpu_params) };
        if context.is_null() {
            Err("Failed to create context (CPU attempt failed)".to_string())
        } else {
            eprintln!("✓ CPU-only context created successfully");
            Ok(Self { ptr: context })
        }
    }

    pub fn get_logits_ith(&self, i: i32) -> *mut f32 {
        // SAFETY: Null return is checked immediately after this call
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
        // SAFETY: Pointer was allocated by llama.cpp FFI and is guaranteed non-null after construction
        unsafe {
            ffi::llama_free(self.ptr);
        }
    }
}

// Gemma-3 chat template
const GEMMA3_TEMPLATE: &str = "{% for message in messages %}{% if message['role'] == 'user' %}{{'<start_of_turn>user\n' + message['content'] + '<end_of_turn>\n'}}{% elif message['role'] == 'assistant' %}{{'<start_of_turn>model\n' + message['content'] + '<end_of_turn>\n'}}{% endif %}{% endfor %}{% if add_generation_prompt %}<start_of_turn>model\n{% endif %}";

// Mistral V3 chat template with BOS token and tool placeholders
// BOS token is critical for Mistral v3 - injected via {{ bos_token }}
// Tool tokens included as placeholders for future function calling
const MISTRAL_V3_TEMPLATE: &str = "{{ bos_token }}{% for message in messages %}{% if message['role'] == 'system' %}[SYSTEM_PROMPT]{{ message['content'] }}[/SYSTEM_PROMPT]{% elif message['role'] == 'user' %}[INST] {{ message['content'] }} [/INST]{% elif message['role'] == 'assistant' %}{{ message['content'] }}</s>{% endif %}{% endfor %}";

// Qwen3 ChatML format template
// Uses <|im_start|>role\ncontent<|im_end|> format
const QWEN3_TEMPLATE: &str = "{% for message in messages %}{% if message['role'] == 'system' %}<|im_start|>system\n{{ message['content'] }}<|im_end|>\n{% elif message['role'] == 'user' %}<|im_start|>user\n{{ message['content'] }}<|im_end|>\n{% elif message['role'] == 'assistant' %}<|im_start|>assistant\n{{ message['content'] }}<|im_end|>\n{% endif %}{% endfor %}{% if add_generation_prompt %}<|im_start|>assistant\n{% endif %}";

// GLM-4 chat template
// Uses [gMASK]<sop> prefix and <|system|>, <|user|>, <|assistant|>, <|observation|> role tokens
// Note: GLM-4.7-Flash is trained as a Code Interpreter but we use it for general chat.
// Only call tools when explicitly needed - prefer direct answers for simple questions.
const GLM4_TEMPLATE: &str = "[gMASK]<sop>{% for message in messages %}{% if message['role'] == 'system' %}<|system|>\n{{ message['content'] }}{% elif message['role'] == 'user' %}<|user|>\n{{ message['content'] }}{% elif message['role'] == 'assistant' %}<|assistant|>\n{{ message['content'] }}{% elif message['role'] == 'observation' %}<|observation|>{{ message['content'] }}{% endif %}{% endfor %}{% if add_generation_prompt %}<|assistant|>\n{% endif %}";

#[cfg(not(target_env = "musl"))]
pub fn apply_chat_template(
    messages: &[ffi::llama_chat_message],
    add_assistant: bool,
) -> Result<Vec<u8>, String> {
    // Default to Gemma-3 for backward compatibility
    apply_chat_template_with_format(messages, add_assistant, ModelFormat::Gemma3)
}

#[cfg(not(target_env = "musl"))]
pub fn apply_chat_template_with_format(
    messages: &[ffi::llama_chat_message],
    add_assistant: bool,
    format: ModelFormat,
) -> Result<Vec<u8>, String> {
    let template = match format {
        ModelFormat::Gemma3 => GEMMA3_TEMPLATE,
        ModelFormat::MistralV3 => MISTRAL_V3_TEMPLATE,
        ModelFormat::Qwen3 => QWEN3_TEMPLATE,
        ModelFormat::GLM4 => GLM4_TEMPLATE,
    };

    if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
        eprintln!("Using chat template format: {:?}", format);
    }

    let template_cstring =
        CString::new(template).map_err(|e| format!("Failed to create template CString: {}", e))?;

    let mut buf = vec![0u8; 64 * 1024];
    loop {
        // SAFETY: CString is valid null-terminated UTF-8; pointer is valid for the duration of the call
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
        // SAFETY: CString is valid null-terminated UTF-8; pointer is valid for the duration of the call
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
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
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

/// Convert a token to raw bytes (may be incomplete UTF-8)
#[cfg(not(target_env = "musl"))]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn token_to_bytes(
    vocab: *const ffi::llama_vocab,
    token: ffi::llama_token,
    special: bool,
) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; 32];
    loop {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
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
            return Ok(buf);
        }
        let need = n.checked_neg().unwrap_or((buf.len() * 2) as i32) as usize;
        buf.resize(need, 0);
    }
}

/// Convert a token to a UTF-8 string piece
/// Note: BPE tokens may represent incomplete UTF-8 sequences.
/// Use `token_to_bytes` and buffer for streaming to handle this correctly.
#[cfg(not(target_env = "musl"))]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn token_to_piece(
    vocab: *const ffi::llama_vocab,
    token: ffi::llama_token,
    special: bool,
) -> Result<String, String> {
    let bytes = token_to_bytes(vocab, token, special)?;
    String::from_utf8(bytes).map_err(|e| format!("UTF-8 conversion error: {}", e))
}

#[cfg(not(target_env = "musl"))]
pub fn batch_get_one(tokens: &[ffi::llama_token]) -> ffi::llama_batch {
    // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
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
    // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
    let batch = unsafe {
        ffi::llama_batch_get_one(
            tokens.as_ptr() as *mut ffi::llama_token,
            tokens.len() as i32,
        )
    };

    // Set logits=1 on the last token if requested (crucial for sampling)
    if request_logits_on_last && !tokens.is_empty() && !batch.logits.is_null() {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
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
    // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
    let batch = unsafe {
        ffi::llama_batch_get_one(
            tokens.as_ptr() as *mut ffi::llama_token,
            tokens.len() as i32,
        )
    };

    // Check if position array is available and adjust positions
    if !batch.pos.is_null() {
        for i in 0..tokens.len() {
            // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
            unsafe {
                *batch.pos.add(i) = pos_offset + i as i32;
            }
        }
    }

    // Set logits=1 on the last token if requested (crucial for sampling)
    if request_logits_on_last && !tokens.is_empty() && !batch.logits.is_null() {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
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
    // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
    let mut batch = unsafe {
        ffi::llama_batch_init(
            tokens.len() as i32,
            0, // embd = 0 for token mode
            1, // n_seq_max = 1
        )
    };

    // Fill batch arrays - all arrays are guaranteed allocated by llama_batch_init
    for (i, &token) in tokens.iter().enumerate() {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
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
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
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
    // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
    unsafe {
        ffi::llama_batch_free(*batch);
    }
}

#[cfg(not(target_env = "musl"))]
pub fn decode_batch(ctx: &LlamaContext, batch: ffi::llama_batch) -> Result<(), String> {
    // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
    let result = unsafe { ffi::llama_decode(ctx.ptr, batch) };
    if result != 0 {
        Err(format!("llama_decode failed with code: {}", result))
    } else {
        Ok(())
    }
}

#[cfg(not(target_env = "musl"))]
pub fn get_logits_ith(ctx: &LlamaContext, i: i32) -> *mut f32 {
    // SAFETY: Null return is checked immediately after this call
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
        // SAFETY: Null return is checked immediately after this call
        let sampler = unsafe { ffi::llama_sampler_chain_init(chain_params) };
        if sampler.is_null() {
            Err("Failed to create sampler chain".to_string())
        } else {
            Ok(Self { ptr: sampler })
        }
    }

    pub fn add_temp(&self, temp: f32) {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        let temp_sampler = unsafe { ffi::llama_sampler_init_temp(temp) };
        if !temp_sampler.is_null() {
            unsafe { ffi::llama_sampler_chain_add(self.ptr, temp_sampler) };
        }
    }

    pub fn add_greedy(&self) {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        let greedy_sampler = unsafe { ffi::llama_sampler_init_greedy() };
        if !greedy_sampler.is_null() {
            unsafe { ffi::llama_sampler_chain_add(self.ptr, greedy_sampler) };
        }
    }

    pub fn add_top_k(&self, k: i32) {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        let top_k_sampler = unsafe { ffi::llama_sampler_init_top_k(k) };
        if !top_k_sampler.is_null() {
            unsafe { ffi::llama_sampler_chain_add(self.ptr, top_k_sampler) };
        }
    }

    pub fn add_top_p(&self, p: f32, min_keep: usize) {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        let top_p_sampler = unsafe { ffi::llama_sampler_init_top_p(p, min_keep) };
        if !top_p_sampler.is_null() {
            unsafe { ffi::llama_sampler_chain_add(self.ptr, top_p_sampler) };
        }
    }

    /// Add logit bias constraints for constrained decoding (e.g., TØR-G)
    ///
    /// Each bias entry sets a token's logit to the specified value.
    /// Use `f32::NEG_INFINITY` to completely disable a token.
    pub fn add_logit_bias(&self, n_vocab: i32, biases: &[ffi::llama_logit_bias]) {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        let bias_sampler = unsafe {
            ffi::llama_sampler_init_logit_bias(n_vocab, biases.len() as i32, biases.as_ptr())
        };
        if !bias_sampler.is_null() {
            unsafe { ffi::llama_sampler_chain_add(self.ptr, bias_sampler) };
        }
    }

    /// Add adaptive-p sampler for dynamic probability adjustment (new in b7785)
    ///
    /// Adaptive-P dynamically adjusts sampling probability based on model confidence.
    /// - `target`: Target cumulative probability (0.0-1.0), e.g., 0.9
    /// - `decay`: How quickly to adapt (0.0-1.0), higher = faster adaptation
    /// - `seed`: Random seed for reproducibility
    ///
    /// This produces more natural text by allowing the model to be more deterministic
    /// when confident and more exploratory when uncertain.
    pub fn add_adaptive_p(&self, target: f32, decay: f32, seed: u32) {
        let _ = (decay, seed);
        // llama_sampler_init_adaptive_p is not available in the current API surface.
        // min_p provides a close dynamic-probability alternative with supported ABI.
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        let adaptive_sampler = unsafe { ffi::llama_sampler_init_min_p(target, 1) };
        if !adaptive_sampler.is_null() {
            unsafe { ffi::llama_sampler_chain_add(self.ptr, adaptive_sampler) };
        }
    }

    /// Add distribution sampler for final token selection
    ///
    /// Unlike greedy sampling, this samples from the probability distribution,
    /// providing stochastic output even after other samplers have filtered candidates.
    pub fn add_dist(&self, seed: u32) {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        let dist_sampler = unsafe { ffi::llama_sampler_init_dist(seed) };
        if !dist_sampler.is_null() {
            unsafe { ffi::llama_sampler_chain_add(self.ptr, dist_sampler) };
        }
    }

    /// Add DRY (Don't Repeat Yourself) sampler to prevent repetition loops.
    ///
    /// CRITICAL for GLM-4.7-Flash: Without dry_multiplier ~1.1, the model loops.
    ///
    /// - `vocab`: Model vocabulary for sequence detection
    /// - `n_ctx_train`: Training context size
    /// - `dry_multiplier`: Penalty multiplier (1.1 recommended for GLM-4.7)
    /// - `dry_base`: Base for exponential penalty growth (default 1.75)
    /// - `dry_allowed_length`: Min length before penalty applies (default 2)
    /// - `dry_penalty_last_n`: How many tokens back to check (default 128)
    ///
    /// # Safety
    /// The `vocab` pointer must be valid and point to a valid llama_vocab struct.
    pub unsafe fn add_dry(
        &self,
        vocab: *const ffi::llama_vocab,
        n_ctx_train: i32,
        dry_multiplier: f32,
        dry_base: f32,
        dry_allowed_length: i32,
        dry_penalty_last_n: i32,
    ) {
        // Default sequence breakers that reset the repetition detector
        let mut seq_breakers: [*const std::os::raw::c_char; 4] =
            [c"\n".as_ptr(), c":".as_ptr(), c"\"".as_ptr(), c"*".as_ptr()];

        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        let dry_sampler = unsafe {
            ffi::llama_sampler_init_dry(
                vocab,
                n_ctx_train,
                dry_multiplier,
                dry_base,
                dry_allowed_length,
                dry_penalty_last_n,
                seq_breakers.as_mut_ptr(),
                seq_breakers.len(),
            )
        };
        if !dry_sampler.is_null() {
            unsafe { ffi::llama_sampler_chain_add(self.ptr, dry_sampler) };
        }
    }

    pub fn sample(&self, ctx: &LlamaContext, idx: i32) -> ffi::llama_token {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        unsafe { ffi::llama_sampler_sample(self.ptr, ctx.ptr, idx) }
    }

    pub fn accept(&self, token: ffi::llama_token) {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        unsafe { ffi::llama_sampler_accept(self.ptr, token) };
    }
}

// SAFETY: Access is serialized through Mutex in LlamaModel/LlamaContext
#[cfg(not(target_env = "musl"))]
unsafe impl Send for LlamaSampler {}

#[cfg(not(target_env = "musl"))]
impl Drop for LlamaSampler {
    fn drop(&mut self) {
        // SAFETY: Pointer was allocated by llama.cpp FFI and is guaranteed non-null after construction
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

/// Dry sampling configuration for repetition prevention
#[derive(Debug, Clone, Copy)]
pub struct DrySamplingConfig {
    /// Penalty multiplier (1.1 recommended for GLM-4.7 to prevent looping)
    pub multiplier: f32,
    /// Base for exponential penalty growth (default 1.75)
    pub base: f32,
    /// Min repetition length before penalty applies (default 2)
    pub allowed_length: i32,
    /// How many tokens back to check for repetitions (default 128)
    pub penalty_last_n: i32,
}

impl Default for DrySamplingConfig {
    fn default() -> Self {
        Self {
            multiplier: 0.0, // Disabled by default
            base: 1.75,
            allowed_length: 2,
            penalty_last_n: 128,
        }
    }
}

impl DrySamplingConfig {
    /// Create a config for GLM-4.7-Flash with recommended dry_multiplier 1.1
    pub fn for_glm() -> Self {
        Self {
            multiplier: 1.1,
            ..Default::default()
        }
    }

    /// Check if dry sampling is enabled
    pub fn is_enabled(&self) -> bool {
        self.multiplier > 0.0
    }
}

/// Create a sampler chain with optional dry sampling for repetition prevention
///
/// Dry sampling is critical for GLM-4.7-Flash to prevent looping.
/// Use `DrySamplingConfig::for_glm()` when working with GLM models.
///
/// # Safety
/// The `vocab` pointer must be valid and point to a valid llama_vocab struct.
#[cfg(not(target_env = "musl"))]
pub unsafe fn create_sampler_chain_with_dry(
    temp: f32,
    top_p: f32,
    top_k: i32,
    _seed: u32,
    dry_config: DrySamplingConfig,
    vocab: *const ffi::llama_vocab,
    n_ctx_train: i32,
) -> Result<LlamaSampler, String> {
    // Clamp params to reasonable ranges
    let top_k = if top_k < 1 { 40 } else { top_k };
    let top_p = top_p.clamp(0.1, 1.0);
    let temp = temp.max(0.0);

    let sampler = LlamaSampler::new_chain(false)?;

    // Add dry sampling early if enabled (before other filters)
    if dry_config.is_enabled() {
        sampler.add_dry(
            vocab,
            n_ctx_train,
            dry_config.multiplier,
            dry_config.base,
            dry_config.allowed_length,
            dry_config.penalty_last_n,
        );
        if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
            eprintln!("Sampler: dry_multiplier={}", dry_config.multiplier);
        }
    }

    if temp <= 0.0 {
        sampler.add_greedy();
        if LLAMA_LOGGING_ENABLED.load(Ordering::Relaxed) {
            eprintln!("Sampler: greedy (deterministic)");
        }
    } else {
        if top_k > 0 {
            sampler.add_top_k(top_k);
        }
        if top_p < 1.0 {
            sampler.add_top_p(top_p, 1);
        }
        sampler.add_temp(temp);
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
    // SAFETY: Null return is checked immediately after this call
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_model_format_mistral() {
        assert_eq!(
            detect_model_format("Ministral-3-3B-Instruct"),
            ModelFormat::MistralV3
        );
        assert_eq!(
            detect_model_format("ministral-8b-instruct-2512"),
            ModelFormat::MistralV3
        );
        assert_eq!(
            detect_model_format("Mistral-Small-3.2"),
            ModelFormat::MistralV3
        );
        assert_eq!(
            detect_model_format("/path/to/mistral-7b.gguf"),
            ModelFormat::MistralV3
        );
    }

    #[test]
    fn test_detect_model_format_gemma() {
        assert_eq!(detect_model_format("gemma-3-4b"), ModelFormat::Gemma3);
        assert_eq!(
            detect_model_format("google/gemma-3-27b-it"),
            ModelFormat::Gemma3
        );
    }

    #[test]
    fn test_detect_model_format_unknown_defaults_to_qwen3() {
        // Unknown models should default to Qwen3 (TØRG-compatible)
        assert_eq!(detect_model_format("llama-3.2-3b"), ModelFormat::Qwen3);
        assert_eq!(detect_model_format("phi-3-mini"), ModelFormat::Qwen3);
        assert_eq!(detect_model_format("deepseek-coder"), ModelFormat::Qwen3);
        assert_eq!(detect_model_format("unknown-model"), ModelFormat::Qwen3);
    }

    #[test]
    fn test_detect_model_format_qwen() {
        assert_eq!(detect_model_format("qwen2.5-7b"), ModelFormat::Qwen3);
        assert_eq!(detect_model_format("Qwen3-0.6B"), ModelFormat::Qwen3);
        assert_eq!(
            detect_model_format("/path/to/Qwen3-0.6B-Q8_0.gguf"),
            ModelFormat::Qwen3
        );
    }

    #[test]
    fn test_detect_model_format_case_insensitive() {
        assert_eq!(detect_model_format("MINISTRAL-3B"), ModelFormat::MistralV3);
        assert_eq!(detect_model_format("MiStRaL-NeMo"), ModelFormat::MistralV3);
        assert_eq!(detect_model_format("QWEN3-1B"), ModelFormat::Qwen3);
    }

    #[test]
    fn test_model_format_default() {
        assert_eq!(ModelFormat::default(), ModelFormat::Qwen3);
    }

    #[test]
    fn test_gemma3_template_contains_markers() {
        assert!(GEMMA3_TEMPLATE.contains("<start_of_turn>"));
        assert!(GEMMA3_TEMPLATE.contains("<end_of_turn>"));
    }

    #[test]
    fn test_mistral_v3_template_contains_markers() {
        assert!(MISTRAL_V3_TEMPLATE.contains("{{ bos_token }}"));
        assert!(MISTRAL_V3_TEMPLATE.contains("[SYSTEM_PROMPT]"));
        assert!(MISTRAL_V3_TEMPLATE.contains("[/SYSTEM_PROMPT]"));
        assert!(MISTRAL_V3_TEMPLATE.contains("[INST]"));
        assert!(MISTRAL_V3_TEMPLATE.contains("[/INST]"));
        assert!(MISTRAL_V3_TEMPLATE.contains("</s>"));
    }

    #[test]
    fn test_qwen3_template_contains_markers() {
        assert!(QWEN3_TEMPLATE.contains("<|im_start|>"));
        assert!(QWEN3_TEMPLATE.contains("<|im_end|>"));
        assert!(QWEN3_TEMPLATE.contains("system"));
        assert!(QWEN3_TEMPLATE.contains("user"));
        assert!(QWEN3_TEMPLATE.contains("assistant"));
    }

    #[test]
    fn test_glm4_template_contains_markers() {
        assert!(GLM4_TEMPLATE.contains("[gMASK]<sop>"));
        assert!(GLM4_TEMPLATE.contains("<|system|>"));
        assert!(GLM4_TEMPLATE.contains("<|user|>"));
        assert!(GLM4_TEMPLATE.contains("<|assistant|>"));
        assert!(GLM4_TEMPLATE.contains("<|observation|>"));
    }

    #[test]
    fn test_glm4_observation_role_no_trailing_newline() {
        // CRITICAL: GLM-4 expects content immediately after <|observation|>
        // with NO newline between the token and the content.
        // This is essential for proper tool result handling.
        let observation_pattern = "<|observation|>{{ message['content'] }}";
        assert!(
            GLM4_TEMPLATE.contains(observation_pattern),
            "Observation role must not have newline after token. Expected: {}",
            observation_pattern
        );
        // Ensure there's no newline variant
        assert!(
            !GLM4_TEMPLATE.contains("<|observation|>\n{{ message['content'] }}"),
            "Observation role must NOT have newline after <|observation|> token"
        );
    }

    #[test]
    fn test_detect_model_format_glm() {
        assert_eq!(detect_model_format("GLM-4.7-Flash"), ModelFormat::GLM4);
        assert_eq!(detect_model_format("glm-4-9b"), ModelFormat::GLM4);
        assert_eq!(
            detect_model_format("bartowski/GLM-4.7-Flash-GGUF"),
            ModelFormat::GLM4
        );
    }

    #[test]
    fn test_dry_sampling_config_defaults() {
        let config = DrySamplingConfig::default();
        assert_eq!(config.multiplier, 0.0); // Disabled by default
        assert!(!config.is_enabled());

        let glm_config = DrySamplingConfig::for_glm();
        assert_eq!(glm_config.multiplier, 1.1); // GLM requires 1.1
        assert!(glm_config.is_enabled());
    }
}
