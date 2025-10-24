#![cfg(all(target_os = "linux", target_arch = "aarch64"))]

use crate::error::{Result, SnpeError};
use libloading::{Library, Symbol};
use std::path::{Path, PathBuf};
use std::sync::Once;

static INIT: Once = Once::new();
static mut SNPE_LIB: Option<Library> = None;

pub fn load_snpe() -> Result<&'static Library> {
    unsafe {
        INIT.call_once(|| match find_and_load_library() {
            Ok(lib) => {
                log::info!("SNPE library loaded successfully");
                SNPE_LIB = Some(lib);
            }
            Err(e) => {
                log::warn!("Failed to load SNPE library: {}", e);
            }
        });

        SNPE_LIB.as_ref().ok_or_else(|| SnpeError::SdkNotFound)
    }
}

fn find_and_load_library() -> Result<Library> {
    let search_paths = get_search_paths();

    for path in &search_paths {
        let lib_path = path.join("libSNPE.so");
        if lib_path.exists() {
            log::info!("Found SNPE library at: {}", lib_path.display());
            return unsafe { Library::new(&lib_path) }
                .map_err(|e| SnpeError::LibraryLoadFailed(e.to_string()));
        }
    }

    Err(SnpeError::SdkNotFound)
}

fn get_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(snpe_root) = std::env::var("SNPE_ROOT") {
        paths.push(PathBuf::from(snpe_root).join("lib").join("aarch64-linux"));
        paths.push(PathBuf::from(snpe_root).join("lib"));
    }

    paths.push(PathBuf::from("/opt/snpe/lib/aarch64-linux"));
    paths.push(PathBuf::from("/opt/snpe/lib"));
    paths.push(PathBuf::from("/usr/local/lib"));
    paths.push(PathBuf::from("/usr/lib"));

    if let Ok(ld_library_path) = std::env::var("LD_LIBRARY_PATH") {
        for path in ld_library_path.split(':') {
            if !path.is_empty() {
                paths.push(PathBuf::from(path));
            }
        }
    }

    paths
}

pub fn is_snpe_available() -> bool {
    load_snpe().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_paths() {
        let paths = get_search_paths();
        assert!(!paths.is_empty());
        assert!(
            paths
                .iter()
                .any(|p| p.to_string_lossy().contains("/opt/snpe"))
        );
    }

    #[test]
    fn test_availability_check() {
        let available = is_snpe_available();
        log::info!("SNPE available: {}", available);
    }
}
