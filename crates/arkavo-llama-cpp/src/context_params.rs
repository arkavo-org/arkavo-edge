//! Sizing of text-generation contexts.
//!
//! Every figure that depends on the context window (the KV cache the loader
//! allocates, the generation clamp, the planner's idea of how much prompt
//! fits) is derived here, so the `ARKAVO_N_CTX` override and the default
//! scaling cannot drift apart between callers.

/// Largest window the default scaling hands out. A Gemma 4 12B KV cache at
/// this length is already several hundred MB per context, and a swarm runs
/// one context per agent process.
pub const DEFAULT_CONTEXT_LENGTH_CEILING: u32 = 16_384;

/// Window used when the model reports a trained context outside the range
/// any real GGUF uses, which indicates corrupt or missing metadata.
const UNTRUSTED_METADATA_CONTEXT_LENGTH: u32 = 8_192;

/// Window used on devices that cannot hold a desktop-sized KV cache.
const CONSTRAINED_CONTEXT_LENGTH: u32 = 2_048;

/// Hardware class that decides how large a context the device can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    /// Qualcomm Adreno under Vulkan: tiny batches, or the driver faults.
    Adreno,
    /// Raspberry Pi class: four cores or fewer.
    LowPower,
    /// Desktop or workstation.
    Standard,
}

/// Which backend the context will compute on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Gpu,
    Cpu,
}

/// Window and batch sizes for one context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextShape {
    pub n_ctx: u32,
    pub n_batch: u32,
    pub n_ubatch: u32,
}

/// Classify the device this process runs on.
pub fn device_class() -> DeviceClass {
    let is_apple_silicon = cfg!(all(target_arch = "aarch64", target_os = "macos"));
    let is_adreno = std::env::var("GGML_VK_MAX_BATCH").is_ok()
        || (cfg!(target_arch = "aarch64") && !is_apple_silicon);
    if is_adreno {
        return DeviceClass::Adreno;
    }

    let num_cores = std::thread::available_parallelism()
        .map(std::num::NonZero::get)
        .unwrap_or(8);
    let is_low_power = std::env::var("ARKAVO_RASPBERRY_PI")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(num_cores <= 4);
    if is_low_power {
        DeviceClass::LowPower
    } else {
        DeviceClass::Standard
    }
}

/// The `ARKAVO_N_CTX` override, when set to a usable number.
pub fn context_length_override() -> Option<u32> {
    parse_context_length_override(std::env::var("ARKAVO_N_CTX").ok().as_deref())
}

/// Zero is rejected because llama.cpp reads it as "use the trained context",
/// which for a 128K model is the allocation the override exists to avoid.
fn parse_context_length_override(raw: Option<&str>) -> Option<u32> {
    raw.and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|&n| n > 0)
}

/// Default window for a model trained on `trained_ctx` tokens.
///
/// Small models keep their full window; larger ones are scaled down because
/// KV cache memory grows linearly with the window.
pub fn scaled_context_length(trained_ctx: u32) -> u32 {
    if !(512..=LARGEST_TRAINED_CONTEXT).contains(&trained_ctx) {
        UNTRUSTED_METADATA_CONTEXT_LENGTH
    } else if trained_ctx <= 8_192 {
        trained_ctx
    } else if trained_ctx <= 32_768 {
        trained_ctx / 2
    } else {
        (trained_ctx / 4).min(DEFAULT_CONTEXT_LENGTH_CEILING)
    }
}

/// Window for a model given an explicit override and device class.
pub fn resolve_context_length(
    trained_ctx: u32,
    override_ctx: Option<u32>,
    device: DeviceClass,
) -> u32 {
    match (override_ctx, device) {
        (Some(n_ctx), _) => n_ctx,
        (None, DeviceClass::Adreno | DeviceClass::LowPower) => CONSTRAINED_CONTEXT_LENGTH,
        (None, DeviceClass::Standard) => scaled_context_length(trained_ctx),
    }
}

/// Window the loader requests for a model trained on `trained_ctx` tokens in
/// this process: the override if set, otherwise the device default.
///
/// llama.cpp rounds the request up to a multiple of 256, so a live context
/// may report slightly more; it never reports less.
pub fn configured_context_length(trained_ctx: u32) -> u32 {
    resolve_context_length(trained_ctx, context_length_override(), device_class())
}

/// Largest trained context the scaling accepts as genuine metadata.
const LARGEST_TRAINED_CONTEXT: u32 = 1_048_576;

/// Largest window the loader gives any model in this process.
///
/// A planner that only knows a model's name, and whose model is not loaded
/// yet, bounds its estimate with this so it never plans for more context
/// than the loader will allocate.
pub fn largest_configured_context_length() -> u32 {
    configured_context_length(LARGEST_TRAINED_CONTEXT)
}

/// Window and batch sizes for a context on `backend`.
pub fn resolve_context_shape(
    trained_ctx: u32,
    override_ctx: Option<u32>,
    device: DeviceClass,
    backend: Backend,
) -> ContextShape {
    let n_ctx = resolve_context_length(trained_ctx, override_ctx, device);
    let (n_batch, n_ubatch) = match (override_ctx, device, backend) {
        (Some(n_ctx), _, _) => ((n_ctx / 16).clamp(16, 2048), (n_ctx / 32).clamp(16, 512)),
        (None, DeviceClass::Adreno, Backend::Gpu) => (16, 16),
        (None, DeviceClass::Adreno, Backend::Cpu) | (None, DeviceClass::LowPower, _) => (512, 256),
        (None, DeviceClass::Standard, _) => (2048, 512),
    };
    ContextShape {
        n_ctx,
        n_batch,
        n_ubatch,
    }
}

/// Parameters for a single-sequence text-generation context.
///
/// `swa_full` is off: llama.cpp defaults it to on, which sizes the cache of
/// every sliding-window layer to the whole context. Gemma 4 12B has 40 such
/// layers with a 1024-token window, so at a 16K context they hold sixteen
/// times the state attention can ever read. With it off each of those layers
/// keeps `n_swa + n_ubatch` cells and recycles the oldest.
///
/// What that gives up is rewinding a sequence to a point more than one
/// window behind its end, because the sliding-window state for the earlier
/// positions has been recycled. Callers here either clear the whole cache per
/// request or rewind only the unaccepted tail of the batch they just decoded
/// (speculative decoding), and cells are only recycled once they fall out of
/// the window as measured before that batch, so both stay exact.
pub fn single_sequence_params(
    shape: ContextShape,
    backend: Backend,
    n_threads: i32,
) -> crate::ffi::llama_context_params {
    // SAFETY: returns a default-initialised struct by value.
    let mut params = unsafe { crate::ffi::llama_context_default_params() };
    params.n_ctx = shape.n_ctx;
    params.n_batch = shape.n_batch;
    params.n_ubatch = shape.n_ubatch;
    params.n_seq_max = 1;
    params.n_threads = n_threads;
    params.n_threads_batch = n_threads;
    params.swa_full = false;
    match backend {
        Backend::Gpu => {
            params.offload_kqv = true;
            params.flash_attn_type = crate::ffi::llama_flash_attn_type_LLAMA_FLASH_ATTN_TYPE_AUTO;
        }
        Backend::Cpu => {
            params.offload_kqv = false;
            params.flash_attn_type =
                crate::ffi::llama_flash_attn_type_LLAMA_FLASH_ATTN_TYPE_DISABLED;
        }
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_keeps_small_windows_and_caps_large_ones() {
        assert_eq!(scaled_context_length(4_096), 4_096);
        assert_eq!(scaled_context_length(8_192), 8_192);
        assert_eq!(scaled_context_length(32_768), 16_384);
        assert_eq!(scaled_context_length(40_960), 10_240);
        assert_eq!(scaled_context_length(131_072), 16_384);
        assert_eq!(scaled_context_length(262_144), 16_384);
    }

    #[test]
    fn scaling_distrusts_implausible_metadata() {
        assert_eq!(scaled_context_length(0), 8_192);
        assert_eq!(scaled_context_length(100), 8_192);
        assert_eq!(scaled_context_length(u32::MAX), 8_192);
    }

    #[test]
    fn default_scaling_never_exceeds_the_ceiling() {
        for trained in [
            512, 2_048, 8_192, 16_384, 32_768, 65_536, 131_072, 1_048_576,
        ] {
            assert!(scaled_context_length(trained) <= DEFAULT_CONTEXT_LENGTH_CEILING);
        }
    }

    #[test]
    fn the_largest_window_is_the_ceiling_or_the_override() {
        assert_eq!(
            resolve_context_length(LARGEST_TRAINED_CONTEXT, None, DeviceClass::Standard),
            DEFAULT_CONTEXT_LENGTH_CEILING
        );
        assert_eq!(
            resolve_context_length(LARGEST_TRAINED_CONTEXT, Some(32_768), DeviceClass::Standard),
            32_768
        );
    }

    #[test]
    fn override_wins_on_every_device() {
        for device in [
            DeviceClass::Adreno,
            DeviceClass::LowPower,
            DeviceClass::Standard,
        ] {
            assert_eq!(resolve_context_length(131_072, Some(4_096), device), 4_096);
        }
    }

    #[test]
    fn constrained_devices_get_a_small_window() {
        assert_eq!(
            resolve_context_length(131_072, None, DeviceClass::LowPower),
            2_048
        );
        assert_eq!(
            resolve_context_length(131_072, None, DeviceClass::Adreno),
            2_048
        );
        assert_eq!(
            resolve_context_length(131_072, None, DeviceClass::Standard),
            16_384
        );
    }

    #[test]
    fn override_parsing_rejects_unusable_values() {
        assert_eq!(parse_context_length_override(Some("8192")), Some(8_192));
        assert_eq!(parse_context_length_override(Some(" 4096 ")), Some(4_096));
        assert_eq!(parse_context_length_override(Some("0")), None);
        assert_eq!(parse_context_length_override(Some("-1")), None);
        assert_eq!(parse_context_length_override(Some("lots")), None);
        assert_eq!(parse_context_length_override(None), None);
    }

    #[test]
    fn batch_sizes_follow_the_device() {
        let standard = resolve_context_shape(131_072, None, DeviceClass::Standard, Backend::Gpu);
        assert_eq!((standard.n_batch, standard.n_ubatch), (2_048, 512));

        let adreno_gpu = resolve_context_shape(131_072, None, DeviceClass::Adreno, Backend::Gpu);
        assert_eq!((adreno_gpu.n_batch, adreno_gpu.n_ubatch), (16, 16));

        let adreno_cpu = resolve_context_shape(131_072, None, DeviceClass::Adreno, Backend::Cpu);
        assert_eq!((adreno_cpu.n_batch, adreno_cpu.n_ubatch), (512, 256));

        let overridden =
            resolve_context_shape(131_072, Some(4_096), DeviceClass::Standard, Backend::Gpu);
        assert_eq!(overridden.n_ctx, 4_096);
        assert_eq!((overridden.n_batch, overridden.n_ubatch), (256, 128));
    }

    /// Regression: llama.cpp defaults `swa_full` to true, which made every
    /// sliding-window layer allocate the whole context.
    #[test]
    fn single_sequence_contexts_do_not_allocate_full_size_swa() {
        let shape = resolve_context_shape(131_072, None, DeviceClass::Standard, Backend::Gpu);
        for backend in [Backend::Gpu, Backend::Cpu] {
            let params = single_sequence_params(shape, backend, 8);
            assert!(!params.swa_full, "{backend:?} context keeps full-size SWA");
            assert_eq!(params.n_seq_max, 1);
            assert_eq!(params.n_ctx, 16_384);
        }
    }

    #[test]
    fn backend_selects_offload_and_flash_attention() {
        let shape = resolve_context_shape(8_192, None, DeviceClass::Standard, Backend::Gpu);
        let gpu = single_sequence_params(shape, Backend::Gpu, 4);
        assert!(gpu.offload_kqv);
        assert_eq!(
            gpu.flash_attn_type,
            crate::ffi::llama_flash_attn_type_LLAMA_FLASH_ATTN_TYPE_AUTO
        );
        let cpu = single_sequence_params(shape, Backend::Cpu, 4);
        assert!(!cpu.offload_kqv);
        assert_eq!(
            cpu.flash_attn_type,
            crate::ffi::llama_flash_attn_type_LLAMA_FLASH_ATTN_TYPE_DISABLED
        );
        assert_eq!(cpu.n_threads, 4);
    }
}
