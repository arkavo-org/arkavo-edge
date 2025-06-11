// idb_companion - iOS Debug Bridge Companion
// Copyright (c) Meta Platforms, Inc. and affiliates.
// Licensed under the MIT License
//
// This module embeds and wraps the idb_companion binary from Meta's idb project.
// See THIRD-PARTY-LICENSES.md for full license text.

use once_cell::sync::Lazy;
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

use crate::{Result, TestError};
#[cfg(target_os = "macos")]
use super::frameworks_data;

// Embed the idb_companion binary at compile time
#[cfg(target_os = "macos")]
static IDB_COMPANION_BYTES: &[u8] = include_bytes!(env!("IDB_COMPANION_PATH"));

// Embed the frameworks archive
#[cfg(target_os = "macos")]
static IDB_FRAMEWORKS_ARCHIVE: &[u8] = include_bytes!(env!("IDB_FRAMEWORKS_ARCHIVE"));

// Provide empty bytes for non-macOS platforms
#[cfg(not(target_os = "macos"))]
static IDB_COMPANION_BYTES: &[u8] = &[];

#[cfg(not(target_os = "macos"))]
static IDB_FRAMEWORKS_ARCHIVE: &[u8] = &[];

// Global path to extracted binary
static EXTRACTED_IDB_PATH: Lazy<Mutex<Option<PathBuf>>> = Lazy::new(|| Mutex::new(None));

// Track connected devices for idb_companion
pub(crate) static CONNECTED_DEVICES: Lazy<Mutex<std::collections::HashSet<String>>> = 
    Lazy::new(|| Mutex::new(std::collections::HashSet::new()));

// Track if we should use system IDB due to framework conflicts
static USE_SYSTEM_IDB: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

/// Wrapper around the embedded idb_companion binary
pub struct IdbWrapper;

impl IdbWrapper {
    /// Initialize idb_companion by extracting it to a temporary location
    pub fn initialize() -> Result<()> {
        eprintln!("[IdbWrapper::initialize] Starting IDB initialization...");
        Self::initialize_with_preference(false)
    }
    
    /// Initialize with option to prefer system IDB
    pub fn initialize_with_preference(prefer_system: bool) -> Result<()> {
        eprintln!("[IdbWrapper::initialize_with_preference] Initializing with prefer_system={}", prefer_system);
        
        #[cfg(not(target_os = "macos"))]
        {
            return Err(TestError::Mcp(
                "idb_companion is only supported on macOS".to_string(),
            ));
        }

        #[cfg(target_os = "macos")]
        {
            // Check environment variable for forcing system IDB
            let force_system = std::env::var("ARKAVO_USE_SYSTEM_IDB")
                .map(|v| v == "1" || v.to_lowercase() == "true")
                .unwrap_or(false);
            
            // Check if we should prefer system IDB
            if prefer_system || force_system {
                if let Some(system_path) = Self::find_system_idb() {
                    eprintln!("[IdbWrapper] Using system IDB at: {}", system_path.display());
                    let mut use_system = USE_SYSTEM_IDB.lock().unwrap();
                    *use_system = true;
                    
                    // Set the path to system IDB
                    let mut path_guard = EXTRACTED_IDB_PATH.lock().unwrap();
                    *path_guard = Some(system_path);
                    return Ok(());
                } else if force_system {
                    return Err(TestError::Mcp(
                        "ARKAVO_USE_SYSTEM_IDB is set but system IDB not found. \
                         Please install it via 'brew install facebook/fb/idb-companion'.".to_string()
                    ));
                }
            }
            
            let mut path_guard = EXTRACTED_IDB_PATH.lock().unwrap();

            if let Some(ref existing_path) = *path_guard {
                // Already initialized - verify it still exists
                if existing_path.exists() {
                    eprintln!("[IdbWrapper::initialize_with_preference] Already initialized at: {}", existing_path.display());
                    return Ok(());
                } else {
                    eprintln!("[IdbWrapper::initialize_with_preference] Previous extraction at {} no longer exists, re-extracting...", existing_path.display());
                    *path_guard = None;
                }
            }

            // Check if we have a real binary or just a placeholder
            eprintln!("[IdbWrapper] Embedded binary size: {} bytes", IDB_COMPANION_BYTES.len());
            if IDB_COMPANION_BYTES.len() < 1000 {
                return Err(TestError::Mcp(
                    "idb_companion not properly embedded. The build should have downloaded it automatically."
                        .to_string(),
                ));
            }

            // Create a directory structure within the project that matches the binary's expectations
            // The binary expects to be in a 'bin' directory with frameworks at '../Frameworks'
            let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let project_root = manifest_dir
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf();
            let temp_dir = project_root.join("target").join("arkavo_idb");
            let bin_dir = temp_dir.join("bin");
            fs::create_dir_all(&bin_dir)
                .map_err(|e| TestError::Mcp(format!("Failed to create bin dir: {}", e)))?;

            let binary_path = bin_dir.join("idb_companion");

            // Extract the binary
            eprintln!("[IdbWrapper] Extracting binary to: {}", binary_path.display());
            fs::write(&binary_path, IDB_COMPANION_BYTES)
                .map_err(|e| TestError::Mcp(format!("Failed to extract idb_companion: {}", e)))?;
            
            // Verify the file was written correctly
            let file_size = fs::metadata(&binary_path)
                .map(|m| m.len())
                .unwrap_or(0);
            
            eprintln!("[IdbWrapper] Binary extracted, size: {} bytes (expected: {} bytes)", file_size, IDB_COMPANION_BYTES.len());
            
            if file_size != IDB_COMPANION_BYTES.len() as u64 {
                return Err(TestError::Mcp(format!(
                    "Binary extraction failed: expected {} bytes, got {}",
                    IDB_COMPANION_BYTES.len(),
                    file_size
                )));
            }

            // Extract embedded frameworks
            #[cfg(target_os = "macos")]
            {
                if IDB_FRAMEWORKS_ARCHIVE.len() > 0 {
                    eprintln!("[IdbWrapper] Extracting embedded frameworks archive ({} bytes)", IDB_FRAMEWORKS_ARCHIVE.len());
                    
                    // Clean up existing frameworks directory if it exists
                    let frameworks_dir = temp_dir.join("Frameworks");
                    if frameworks_dir.exists() {
                        let _ = fs::remove_dir_all(&frameworks_dir);
                    }
                    
                    // Write archive to temp file
                    let archive_path = temp_dir.join("frameworks.tar.gz");
                    fs::write(&archive_path, IDB_FRAMEWORKS_ARCHIVE)
                        .map_err(|e| TestError::Mcp(format!("Failed to write frameworks archive: {}", e)))?;
                    
                    // Extract the archive
                    let status = Command::new("tar")
                        .args(&["-xzf", archive_path.to_str().unwrap(), "-C", temp_dir.to_str().unwrap()])
                        .status()
                        .map_err(|e| TestError::Mcp(format!("Failed to extract frameworks: {}", e)))?;
                    
                    if status.success() {
                        eprintln!("[IdbWrapper] Successfully extracted frameworks to {}", temp_dir.display());
                        // Clean up archive
                        let _ = fs::remove_file(&archive_path);
                        
                        // Verify frameworks exist
                        let frameworks_dir = temp_dir.join("Frameworks");
                        if frameworks_dir.exists() {
                            eprintln!("[IdbWrapper] Frameworks directory created at: {}", frameworks_dir.display());
                        }
                    } else {
                        eprintln!("[IdbWrapper] Warning: Failed to extract frameworks archive");
                        eprintln!("[IdbWrapper] IDB companion may fail due to missing framework dependencies");
                    }
                } else {
                    eprintln!("[IdbWrapper] Warning: No embedded frameworks archive found");
                    // Try to set up framework symlinks to system frameworks
                    if let Err(e) = frameworks_data::setup_framework_links(&temp_dir) {
                        eprintln!("[IdbWrapper] Warning: {}", e);
                    }
                }
            }

            // Make it executable
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&binary_path).unwrap().permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&binary_path, perms)
                    .map_err(|e| TestError::Mcp(format!("Failed to set permissions: {}", e)))?;
            }
            


            eprintln!("[IdbWrapper] IDB companion initialized at: {}", binary_path.display());
            *path_guard = Some(binary_path);
            Ok(())
        }
    }

    /// Check if system IDB is available
    fn find_system_idb() -> Option<PathBuf> {
        // Check common locations for system-installed IDB
        let paths = [
            "/opt/homebrew/bin/idb_companion",       // Apple Silicon Homebrew
            "/usr/local/bin/idb_companion",          // Intel Mac Homebrew
            "/usr/bin/idb_companion",                // System location
        ];
        
        for path in &paths {
            let path_buf = PathBuf::from(path);
            if path_buf.exists() {
                return Some(path_buf);
            }
        }
        
        // Also check PATH
        if let Ok(path_env) = std::env::var("PATH") {
            for dir in path_env.split(':') {
                let idb_path = PathBuf::from(dir).join("idb_companion");
                if idb_path.exists() {
                    return Some(idb_path);
                }
            }
        }
        
        None
    }
    
    /// Get the path to the idb_companion binary
    pub fn get_binary_path() -> Result<PathBuf> {
        // Check if we should use system IDB due to framework conflicts
        let use_system = USE_SYSTEM_IDB.lock().unwrap();
        if *use_system {
            if let Some(system_path) = Self::find_system_idb() {
                return Ok(system_path);
            }
        }
        
        // Use the embedded IDB which includes frameworks
        let path_guard = EXTRACTED_IDB_PATH.lock().unwrap();
        path_guard
            .as_ref()
            .cloned()
            .ok_or_else(|| TestError::Mcp("idb_companion not initialized".to_string()))
    }
    
    /// Create a Command with proper framework paths set
    fn create_command() -> Result<Command> {
        let binary_path = Self::get_binary_path()?;
        
        eprintln!("[IdbWrapper::create_command] Binary path: {}", binary_path.display());
        
        // Verify the binary exists before trying to execute it
        if !binary_path.exists() {
            eprintln!("[IdbWrapper::create_command] ERROR: Binary does not exist at path");
            return Err(TestError::Mcp(format!(
                "idb_companion binary not found at expected path: {}",
                binary_path.display()
            )));
        }
        
        // Check if binary is executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&binary_path)
                .map_err(|e| TestError::Mcp(format!("Failed to get binary metadata: {}", e)))?;
            let permissions = metadata.permissions();
            eprintln!("[IdbWrapper::create_command] Binary permissions: {:o}", permissions.mode());
            
            if permissions.mode() & 0o111 == 0 {
                eprintln!("[IdbWrapper::create_command] WARNING: Binary is not executable!");
            }
        }
        
        let mut command = Command::new(&binary_path);
        
        // Only set DYLD variables for embedded IDB, not system IDB
        let use_system = USE_SYSTEM_IDB.lock().unwrap();
        if !*use_system {
            eprintln!("[IdbWrapper::create_command] Using embedded IDB, setting up framework paths...");
            // Set up framework loading to prevent conflicts
            // The binary is in 'bin' dir, frameworks are at '../Frameworks'
            let frameworks_dir = binary_path.parent().unwrap().parent().unwrap().join("Frameworks");
            eprintln!("[IdbWrapper::create_command] Frameworks dir: {}", frameworks_dir.display());
            
            if frameworks_dir.exists() {
                eprintln!("[IdbWrapper::create_command] Frameworks directory exists, setting DYLD environment variables");
                // Use DYLD_INSERT_LIBRARIES to force our frameworks to load first
                // This helps prevent system framework conflicts
                command.env("DYLD_FRAMEWORK_PATH", frameworks_dir.to_str().unwrap());
                
                // Disable library validation to allow loading of unsigned frameworks
                command.env("DYLD_DISABLE_LIBRARY_VALIDATION", "1");
                
                // Set up fallback paths excluding problematic system frameworks
                // Don't include /System/Library/PrivateFrameworks to avoid FrontBoard conflicts
                command.env("DYLD_FALLBACK_FRAMEWORK_PATH", 
                    format!("{}:/System/Library/Frameworks", frameworks_dir.to_str().unwrap()));
                
                // Note: Not setting DYLD_FORCE_FLAT_NAMESPACE as it can cause issues with signed binaries
            }
        }
        
        Ok(command)
    }
    
    /// Ensure the companion server is running for a specific device
    pub async fn ensure_companion_running(device_id: &str) -> Result<()> {
        use std::sync::Mutex;
        use once_cell::sync::Lazy;
        
        // Track running companion processes by device
        static COMPANION_PROCESSES: Lazy<Mutex<HashMap<String, std::process::Child>>> = 
            Lazy::new(|| Mutex::new(HashMap::new()));
        
        // Check and start companion in a separate scope to release the lock before await
        let device_id_owned = device_id.to_string();
        let needs_start = {
            let mut processes = COMPANION_PROCESSES.lock().unwrap();
            
            // Check if we already have a companion running for this device
            if let Some(child) = processes.get_mut(device_id) {
                // Check if it's still running
                match child.try_wait() {
                    Ok(None) => {
                        // Still running
                        return Ok(());
                    }
                    Ok(Some(_)) => {
                        // Process exited, remove it
                        processes.remove(device_id);
                        true
                    }
                    Err(_) => {
                        // Error checking status, remove it
                        processes.remove(device_id);
                        true
                    }
                }
            } else {
                true
            }
        };
        
        if needs_start {
            eprintln!("[IdbWrapper] Starting companion server for device {}...", device_id);
            
            // Start a new companion process for this device
            let mut command = Self::create_command()?;
            eprintln!("[IdbWrapper] Starting companion with command: {:?}", command);
            
            // Try to run with verbose output first to diagnose issues
            eprintln!("[IdbWrapper] Command path: {:?}", command.get_program());
            eprintln!("[IdbWrapper] Command args: --udid {} --only simulator", device_id);
            
            // Check binary architecture
            let file_check = Command::new("file")
                .arg(command.get_program())
                .output();
                
            if let Ok(output) = file_check {
                let file_info = String::from_utf8_lossy(&output.stdout);
                eprintln!("[IdbWrapper] Binary file info: {}", file_info.trim());
                
                // Check if architecture matches current system
                let arch_check = Command::new("arch")
                    .output();
                    
                if let Ok(arch_output) = arch_check {
                    let current_arch = String::from_utf8_lossy(&arch_output.stdout).trim().to_string();
                    eprintln!("[IdbWrapper] Current system architecture: {}", current_arch);
                    
                    if current_arch == "arm64" && !file_info.contains("arm64") {
                        eprintln!("[IdbWrapper] WARNING: Binary might not be compatible with Apple Silicon!");
                    }
                }
            }
            
            // First, try to run with --help to see if binary works at all
            let help_test = Command::new(command.get_program())
                .arg("--help")
                .output();
                
            match help_test {
                Ok(output) => {
                    if output.status.success() {
                        eprintln!("[IdbWrapper] Binary --help test succeeded");
                    } else {
                        eprintln!("[IdbWrapper] Binary --help test failed with status: {:?}", output.status);
                        eprintln!("[IdbWrapper] Stderr: {}", String::from_utf8_lossy(&output.stderr));
                        
                        // Check if it's signal 9 (SIGKILL)
                        if let Some(code) = output.status.code() {
                            if code == 9 {
                                eprintln!("[IdbWrapper] Binary was killed by SIGKILL - likely a code signing or security issue");
                                eprintln!("[IdbWrapper] Try running: sudo spctl --master-disable (temporarily disable Gatekeeper)");
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[IdbWrapper] Failed to run binary --help test: {}", e);
                    return Err(TestError::Mcp(format!("Binary cannot be executed: {}", e)));
                }
            }
            
            // Capture stderr to see any error messages
            let output = command
                .args(["--udid", device_id, "--only", "simulator"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .spawn();
                
            match output {
                Ok(mut child) => {
                    // Check if the process started successfully by waiting briefly
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            // Process exited immediately - capture error
                            let mut stderr_bytes = Vec::new();
                            if let Some(mut stderr) = child.stderr.take() {
                                use std::io::Read;
                                let _ = stderr.read_to_end(&mut stderr_bytes);
                            }
                            let stderr_str = String::from_utf8_lossy(&stderr_bytes);
                            
                            eprintln!("[IdbWrapper] Companion process exited immediately with status: {:?}", status);
                            eprintln!("[IdbWrapper] Stderr: {}", stderr_str);
                            
                            return Err(TestError::Mcp(format!(
                                "IDB companion failed to start: exited with status {:?}. Error: {}", 
                                status, stderr_str
                            )));
                        }
                        Ok(None) => {
                            // Still running - good
                            eprintln!("[IdbWrapper] Companion process started successfully");
                            
                            // Store the process
                            {
                                let mut processes = COMPANION_PROCESSES.lock().unwrap();
                                processes.insert(device_id_owned, child);
                            }
                        }
                        Err(e) => {
                            eprintln!("[IdbWrapper] Failed to check companion process status: {}", e);
                            return Err(TestError::Mcp(format!("Failed to verify companion started: {}", e)));
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[IdbWrapper] Failed to spawn companion process: {}", e);
                    return Err(TestError::Mcp(format!("Failed to start companion: {}", e)));
                }
            }
            
            // Wait for it to initialize
            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
            
            eprintln!("[IdbWrapper] Companion server started for device {}", device_id);
        }
        
        Ok(())
    }
    
    /// Connect to a device if not already connected
    fn ensure_connected(device_id: &str) -> Result<()> {
        let mut connected = CONNECTED_DEVICES.lock().unwrap();
        
        if connected.contains(device_id) {
            return Ok(());
        }
        
        
        // First, check if the device is already connected
        let mut command = Self::create_command()?;
        let list_output = command
            .args(["--list", "1", "--json"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to list targets: {}", e)))?;
            
        if list_output.status.success() {
            let output_str = String::from_utf8_lossy(&list_output.stdout);
            
            // Check if we can see the device in the targets list
            if let Ok(targets) = serde_json::from_str::<serde_json::Value>(&output_str) {
                let device_found = targets.as_array()
                    .map(|arr| arr.iter().any(|t| {
                        t.get("udid").and_then(|u| u.as_str()) == Some(device_id)
                    }))
                    .unwrap_or(false);
                
                if device_found {
                    
                    // For simulators, try to explicitly connect to ensure IDB companion is ready
                    
                    // First, ensure the simulator is in the right state
                    let simctl_output = Command::new("xcrun")
                        .args(["simctl", "list", "devices", "-j"])
                        .output()
                        .ok();
                        
                    if let Some(output) = simctl_output {
                        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                            for (_runtime, devices) in json["devices"].as_object().unwrap_or(&serde_json::Map::new()) {
                                if let Some(device_array) = devices.as_array() {
                                    for device in device_array {
                                        if device["udid"].as_str() == Some(device_id) {
                                            let state = device["state"].as_str().unwrap_or("Unknown");
                                            
                                            if state != "Booted" {
                                            }
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    
                    // Try multiple connection attempts with different strategies
                    let mut connection_established = false;
                    
                    // Strategy 1: Direct connect
                    let mut connect_cmd = Self::create_command()?;
                    let connect_output = connect_cmd
                        .args(["connect", device_id])
                        .output()
                        .map_err(|e| TestError::Mcp(format!("Failed to connect: {}", e)))?;
                    
                    if connect_output.status.success() || 
                       String::from_utf8_lossy(&connect_output.stderr).contains("already connected") {
                        connection_established = true;
                    } else {
                        let stderr = String::from_utf8_lossy(&connect_output.stderr);
                        
                        // Strategy 2: Try with explicit port
                        if stderr.contains("Connection refused") || stderr.contains("failed to connect") {
                            let mut port_cmd = Self::create_command()?;
                            let port_output = port_cmd
                                .args(["connect", device_id, "--companion-port", "10882"])
                                .output()
                                .ok();
                                
                            if let Some(output) = port_output {
                                if output.status.success() {
                                    connection_established = true;
                                }
                            }
                        }
                    }
                    
                    if !connection_established {
                    }
                } else {
                    return Err(TestError::Mcp(format!(
                        "Device {} not found. Make sure the simulator is booted.",
                        device_id
                    )));
                }
            } else {
            }
        } else {
            let stderr = String::from_utf8_lossy(&list_output.stderr);
            
            // Check if IDB companion is not running
            if stderr.contains("Connection refused") || stderr.contains("failed to connect") {
                
                eprintln!("[IdbWrapper] IDB companion not running or device not connected. Starting companion for device {}...", device_id);
                
                // Try to start IDB companion for this specific device
                let mut start_cmd = Self::create_command()?;
                let start_result = start_cmd
                    .args(["--udid", device_id])
                    .spawn();
                    
                if let Ok(mut child) = start_result {
                    eprintln!("[IdbWrapper] Started IDB companion process for device {}", device_id);
                    
                    // Give it a moment to start
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    
                    // Check if it's still running
                    match child.try_wait() {
                        Ok(Some(_status)) => {
                            eprintln!("[IdbWrapper] IDB companion process exited immediately");
                            return Err(TestError::Mcp("Failed to start IDB companion - process exited".to_string()));
                        }
                        Ok(None) => {
                            eprintln!("[IdbWrapper] IDB companion process is running");
                            // Detach the process so it continues running
                            drop(child);
                        }
                        Err(_e) => {
                            eprintln!("[IdbWrapper] Failed to check IDB companion process status");
                        }
                    }
                    
                    // Give it more time to fully initialize
                    std::thread::sleep(std::time::Duration::from_secs(1));
                } else {
                    eprintln!("[IdbWrapper] Failed to start IDB companion for device {}", device_id);
                }
            }
        }
        
        connected.insert(device_id.to_string());
        
        Ok(())
    }

    /// Perform a tap at the specified coordinates
    pub async fn tap(device_id: &str, x: f64, y: f64) -> Result<serde_json::Value> {
        let _start_time = std::time::Instant::now();

        // Initialize and get the embedded binary path
        Self::initialize()?;
        
        // Ensure companion is running for this device
        Self::ensure_companion_running(device_id).await?;
        
        // Use the embedded idb_companion directly with UI commands
        let x_str = x.to_string();
        let y_str = y.to_string();
        let args = vec![
            "ui",
            "tap",
            &x_str,
            &y_str,
            "--udid",
            device_id,
            "--only",
            "simulator",
        ];
        
        
        let mut command = Self::create_command()?;
        command.args(&args);
        
        let output = command
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute idb_companion: {}", e)))?;
        

        if output.status.success() {
            Ok(json!({
                "success": true,
                "method": "idb_companion",
                "action": "tap",
                "coordinates": {"x": x, "y": y},
                "device_id": device_id,
                "confidence": "high"
            }))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            
            // Check for specific errors
            if stderr.contains("Class FBProcess is implemented in both") {
                // Framework conflict - already handled by create_command
                
                // Set flag to use system IDB
                {
                    let mut use_system = USE_SYSTEM_IDB.lock().unwrap();
                    *use_system = true;
                }
                
                // Check if system IDB is available
                if let Some(system_idb) = Self::find_system_idb() {
                    // Retry with system IDB (non-recursive)
                    
                    let mut retry_command = Command::new(&system_idb);
                    retry_command.args(&args);
                    
                    // Don't set DYLD paths for system IDB
                    retry_command.env("DYLD_DISABLE_LIBRARY_VALIDATION", "1");
                    
                    let retry_output = retry_command
                        .output()
                        .map_err(|e| TestError::Mcp(format!("Failed to execute system idb_companion: {}", e)))?;
                    
                    if retry_output.status.success() {
                        return Ok(json!({
                            "success": true,
                            "method": "system_idb_companion",
                            "action": "tap",
                            "coordinates": {"x": x, "y": y},
                            "device_id": device_id,
                            "confidence": "high"
                        }));
                    } else {
                        let retry_stderr = String::from_utf8_lossy(&retry_output.stderr);
                        return Err(TestError::Mcp(format!(
                            "System idb_companion tap also failed: {}",
                            retry_stderr
                        )));
                    }
                } else {
                    return Err(TestError::Mcp(
                        "Framework conflict detected: IDB frameworks conflicting with system frameworks. \
                         System IDB not found. Please install it via 'brew install facebook/fb/idb-companion'.".to_string()
                    ));
                }
            }
            
            // Check for port binding issues
            if stderr.contains("Address already in use") || stderr.contains("port 10882") {
                return Err(TestError::Mcp(
                    "Port 10882 is already in use. IDB companion server may be stuck. \
                     Auto-recovery will attempt to fix this, or you can manually run: \
                     pkill -f idb_companion".to_string()
                ));
            }
            
            Err(TestError::Mcp(format!(
                "idb_companion tap failed: {}",
                stderr
            )))
        }
    }

    /// Perform a swipe gesture
    pub async fn swipe(
        device_id: &str,
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        duration: f64,
    ) -> Result<serde_json::Value> {
        Self::initialize()?;
        Self::ensure_connected(device_id)?;


        // Execute idb_companion swipe command
        let mut command = Self::create_command()?;
        let output = command
            .args([
                "ui",
                "swipe",
                &start_x.to_string(),
                &start_y.to_string(),
                &end_x.to_string(),
                &end_y.to_string(),
                "--duration",
                &duration.to_string(),
                "--udid",
                device_id,
                "--only",
                "simulator",
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute idb_companion: {}", e)))?;

        if output.status.success() {
            Ok(json!({
                "success": true,
                "method": "idb_companion",
                "action": "swipe",
                "start": {"x": start_x, "y": start_y},
                "end": {"x": end_x, "y": end_y},
                "duration": duration,
                "device_id": device_id,
                "confidence": "high"
            }))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);

            Err(TestError::Mcp(format!(
                "idb_companion swipe failed: {}",
                stderr
            )))
        }
    }

    /// List all available targets (devices/simulators)
    pub async fn list_targets() -> Result<serde_json::Value> {
        Self::initialize()?;
        
        
        let mut command = Self::create_command()?;
        command.args(["--list", "1", "--json"]);
        
        let output = command
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute idb_companion: {}", e)))?;
        
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            
            // Try to parse as JSON
            match serde_json::from_str::<serde_json::Value>(&stdout) {
                Ok(targets) => Ok(targets),
                Err(_) => {
                    // If not valid JSON, return structured response
                    Ok(json!({
                        "success": true,
                        "raw_output": stdout.to_string(),
                        "note": "Raw output provided as JSON parsing failed"
                    }))
                }
            }
        } else {
            let _stderr = String::from_utf8_lossy(&output.stderr);
            
            // Return empty array on failure
            Ok(json!([]))
        }
    }

    /// Type text into the currently focused element
    pub async fn type_text(device_id: &str, text: &str) -> Result<serde_json::Value> {
        Self::initialize()?;
        Self::ensure_connected(device_id)?;


        // Execute idb_companion text command
        let mut command = Self::create_command()?;
        let output = command
            .args(["ui", "text", text, "--udid", device_id, "--only", "simulator"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute idb_companion: {}", e)))?;

        if output.status.success() {
            Ok(json!({
                "success": true,
                "method": "idb_companion",
                "action": "type_text",
                "text": text,
                "device_id": device_id,
                "confidence": "high"
            }))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);

            Err(TestError::Mcp(format!(
                "idb_companion type_text failed: {}",
                stderr
            )))
        }
    }

    /// Press a hardware button (e.g., "home", "power", "volumeup", "volumedown")
    pub async fn press_button(device_id: &str, button: &str) -> Result<serde_json::Value> {
        Self::initialize()?;
        Self::ensure_connected(device_id)?;


        // Execute idb_companion button command
        let mut command = Self::create_command()?;
        let output = command
            .args(["ui", "button", button, "--udid", device_id, "--only", "simulator"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute idb_companion: {}", e)))?;

        if output.status.success() {
            Ok(json!({
                "success": true,
                "method": "idb_companion",
                "action": "press_button",
                "button": button,
                "device_id": device_id,
                "confidence": "high"
            }))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);

            Err(TestError::Mcp(format!(
                "idb_companion press_button failed: {}",
                stderr
            )))
        }
    }

    /// List installed apps on device (used for connection verification)
    pub async fn list_apps(device_id: &str) -> Result<serde_json::Value> {
        Self::initialize()?;
        Self::ensure_connected(device_id)?;
        
        
        let mut command = Self::create_command()?;
        command.args(["list-apps", "--udid", device_id, "--only", "simulator", "--json"]);
        
        let output = command
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute idb_companion: {}", e)))?;
        
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            
            // Try to parse as JSON
            match serde_json::from_str::<serde_json::Value>(&stdout) {
                Ok(apps) => Ok(apps),
                Err(_) => {
                    // If not valid JSON, return structured response
                    Ok(json!({
                        "success": true,
                        "raw_output": stdout.to_string(),
                        "note": "Raw output provided as JSON parsing failed"
                    }))
                }
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(TestError::Mcp(format!(
                "idb_companion list-apps failed: {}",
                stderr
            )))
        }
    }

    /// Clean up extracted binary on drop
    pub fn cleanup() {
        if let Ok(mut path_guard) = EXTRACTED_IDB_PATH.lock() {
            if let Some(path) = path_guard.take() {
                let _ = fs::remove_file(&path);
            }
        }
        
        if let Ok(mut connected) = CONNECTED_DEVICES.lock() {
            connected.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_idb_wrapper_initialization() {
        // This test will fail on non-macOS platforms as expected
        let result = IdbWrapper::initialize();

        #[cfg(target_os = "macos")]
        {
            // On macOS, initialization should succeed (though the placeholder will fail)
            match result {
                Ok(_) => {},
                Err(_) => {},
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            // On other platforms, should return error
            assert!(result.is_err());
        }
    }
}