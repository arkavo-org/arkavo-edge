#![cfg(all(target_os = "linux", target_arch = "aarch64"))]

//! Qualcomm SNPE (Snapdragon Neural Processing Engine) backend for Arkavo Edge
//!
//! Provides hardware-accelerated inference on Qualcomm platforms like Arduino UNO Q (QRB2210)
//! with support for GPU (Adreno), DSP (Hexagon), and CPU runtimes.
//!
//! Architecture:
//! - Runtime detection and initialization via SNPE SDK
//! - DLC (Deep Learning Container) model loading
//! - Multi-target acceleration (GPU > DSP > CPU fallback)
//! - Tensor conversion and inference execution

pub mod accelerator;
pub mod error;
pub mod runtime;
pub mod tensor;

pub use accelerator::{AcceleratorTarget, RuntimeCapabilities};
pub use error::{Result, SnpeError};
pub use runtime::SnpeRuntime;
pub use tensor::{SnpeTensor, TensorShape};

use std::path::{Path, PathBuf};
use std::sync::Once;

static INIT: Once = Once::new();

/// Initialize SNPE library
///
/// Must be called before any other SNPE operations.
/// Safe to call multiple times (uses `Once` guard).
pub fn initialize() -> Result<()> {
    INIT.call_once(|| {
        log::info!("Initializing SNPE runtime");
        if let Err(e) = verify_snpe_sdk() {
            log::error!("SNPE SDK verification failed: {}", e);
        }
    });
    Ok(())
}

/// Verify SNPE SDK installation
fn verify_snpe_sdk() -> Result<()> {
    let snpe_root = std::env::var("SNPE_ROOT").map_err(|_| SnpeError::SdkNotFound)?;

    let snpe_path = Path::new(&snpe_root);
    if !snpe_path.exists() {
        return Err(SnpeError::SdkNotFound);
    }

    let lib_path = snpe_path.join("lib").join("aarch64-linux");
    if !lib_path.exists() {
        return Err(SnpeError::InvalidSdkPath(format!(
            "Expected library path not found: {}",
            lib_path.display()
        )));
    }

    log::info!("SNPE SDK found at: {}", snpe_root);
    Ok(())
}

/// Get SNPE SDK root directory
pub fn get_snpe_root() -> Result<PathBuf> {
    let snpe_root = std::env::var("SNPE_ROOT").map_err(|_| SnpeError::SdkNotFound)?;
    Ok(PathBuf::from(snpe_root))
}

/// Get SNPE library path for dynamic loading
pub fn get_snpe_lib_path() -> Result<PathBuf> {
    let snpe_root = get_snpe_root()?;
    let lib_path = snpe_root.join("lib").join("aarch64-linux");

    if !lib_path.exists() {
        return Err(SnpeError::InvalidSdkPath(format!(
            "Library path does not exist: {}",
            lib_path.display()
        )));
    }

    Ok(lib_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize() {
        let result = initialize();
        assert!(result.is_ok());
    }

    #[test]
    fn test_snpe_root_detection() {
        if std::env::var("SNPE_ROOT").is_ok() {
            let root = get_snpe_root();
            assert!(root.is_ok());

            let lib_path = get_snpe_lib_path();
            assert!(lib_path.is_ok());
        }
    }
}
