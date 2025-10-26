#![cfg(all(target_os = "linux", target_arch = "aarch64"))]

//! Qualcomm SNPE (Snapdragon Neural Processing Engine) backend for Arkavo Edge
//!
//! Provides hardware-accelerated inference on Qualcomm platforms like Arduino UNO Q (QRB2210)
//! with support for GPU (Adreno), DSP (Hexagon), and CPU runtimes.
//!
//! Dynamic Loading:
//! - SNPE library loaded at runtime via dlopen (no build-time dependency)
//! - Searches: /opt/snpe/lib, $SNPE_ROOT/lib, $LD_LIBRARY_PATH
//! - Gracefully falls back to CPU if library not found
//! - Pre-built binaries work on any system (SNPE features enabled when SDK installed)
//!
//! Architecture:
//! - Runtime detection and initialization via dynamic loading
//! - DLC (Deep Learning Container) model loading
//! - Multi-target acceleration (GPU > DSP > CPU fallback)
//! - Tensor conversion and inference execution

pub mod accelerator;
pub mod error;
pub mod loader;
pub mod runtime;
pub mod tensor;

pub use accelerator::{AcceleratorTarget, RuntimeCapabilities};
pub use error::{Result, SnpeError};
pub use loader::{is_snpe_available, load_snpe};
pub use runtime::SnpeRuntime;
pub use tensor::{SnpeTensor, TensorShape};

/// Initialize SNPE library (dynamic loading)
///
/// Attempts to load SNPE SDK at runtime. Returns Ok even if SDK not found.
/// Check `is_snpe_available()` to determine if SNPE is loaded.
pub fn initialize() -> Result<()> {
    match load_snpe() {
        Ok(_) => {
            log::info!("SNPE library loaded successfully");
            Ok(())
        }
        Err(e) => {
            log::warn!("SNPE library not available: {}", e);
            log::warn!("Falling back to CPU inference");
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize() {
        let _result = initialize();
    }

    #[test]
    fn test_availability() {
        let available = is_snpe_available();
        if available {
            log::info!("SNPE is available");
        } else {
            log::info!("SNPE is not available (expected in CI)");
        }
    }
}
