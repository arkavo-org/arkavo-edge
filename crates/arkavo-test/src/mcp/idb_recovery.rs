use crate::{Result, TestError};
use std::net::TcpStream;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct IdbRecovery {
    last_recovery: Arc<Mutex<Option<Instant>>>,
}

impl IdbRecovery {
    pub fn new() -> Self {
        Self {
            last_recovery: Arc::new(Mutex::new(None)),
        }
    }

    /// Check if IDB companion is running
    pub async fn is_companion_running() -> bool {
        #[cfg(target_os = "macos")]
        {
            let output = Command::new("pgrep")
                .arg("-f")
                .arg("idb_companion")
                .output()
                .ok();

            if let Some(output) = output {
                output.status.success()
            } else {
                false
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    /// Check if IDB companion port is accessible
    pub async fn is_companion_port_accessible() -> bool {
        #[cfg(target_os = "macos")]
        {
            // Try to connect to default IDB port
            if let Ok(_) = TcpStream::connect("127.0.0.1:10882") {
                return true;
            }

            // Also check alternative ports
            for port in [10883, 10884, 10885] {
                if let Ok(_) = TcpStream::connect(format!("127.0.0.1:{}", port)) {
                    return true;
                }
            }
            false
        }

        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    /// Kill any stuck IDB companion processes
    pub async fn kill_stuck_processes(&self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            // Use pkill to terminate all idb_companion processes
            let _ = Command::new("pkill")
                .arg("-f")
                .arg("idb_companion")
                .output();

            // Also kill any notifier processes which IDB uses
            let _ = Command::new("pkill").arg("-f").arg("notifier").output();

            // Give processes time to terminate
            tokio::time::sleep(Duration::from_secs(1)).await;

            // Force kill if still running
            if Self::is_companion_running().await {
                let _ = Command::new("pkill")
                    .arg("-9")
                    .arg("-f")
                    .arg("idb_companion")
                    .output();

                // Force kill notifier too
                let _ = Command::new("pkill")
                    .arg("-9")
                    .arg("-f")
                    .arg("notifier")
                    .output();

                tokio::time::sleep(Duration::from_millis(500)).await;
            }

            // Clear any IDB-related lock files
            let _ = Command::new("rm")
                .arg("-f")
                .arg("/tmp/idb_companion.lock")
                .output();

            eprintln!("[IdbRecovery] Cleaned up IDB processes and lock files");

            Ok(())
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok(())
        }
    }

    /// Attempt to recover IDB companion
    pub async fn attempt_recovery(&self) -> Result<()> {
        // Check if we've attempted recovery recently
        let mut last_recovery = self.last_recovery.lock().await;

        if let Some(last_time) = *last_recovery {
            if last_time.elapsed() < Duration::from_secs(30) {
                return Err(TestError::Mcp(
                    "Recovery attempted too recently. Please wait 30 seconds.".to_string(),
                ));
            }
        }

        eprintln!("[IdbRecovery] Starting IDB recovery process...");

        // Check current state
        let companion_running = Self::is_companion_running().await;
        let port_accessible = Self::is_companion_port_accessible().await;

        eprintln!(
            "[IdbRecovery] Current state - Companion running: {}, Port accessible: {}",
            companion_running, port_accessible
        );

        // Special handling for "companion running but not connected" scenario
        if companion_running && !port_accessible {
            eprintln!(
                "[IdbRecovery] Detected companion running but port not accessible - likely stuck"
            );
            eprintln!("[IdbRecovery] Will perform aggressive recovery");

            // Force kill the stuck companion immediately
            eprintln!("[IdbRecovery] Force killing stuck companion process...");
            let _ = Command::new("pkill")
                .arg("-9")
                .arg("-f")
                .arg("idb_companion")
                .output();
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        // Step 1: Kill stuck processes
        self.kill_stuck_processes().await?;

        // Step 2: Clear IDB companion cache and state
        #[cfg(target_os = "macos")]
        {
            let cache_paths = [
                "~/Library/Caches/com.facebook.idb",
                "/tmp/idb*",
                "/var/folders/*/*/T/idb*",
                "/tmp/idb_companion.lock",
                "/tmp/notifier*",
            ];

            for path in &cache_paths {
                eprintln!("[IdbRecovery] Clearing cache: {}", path);
                let expanded = if path.starts_with("~") {
                    let home = std::env::var("HOME").unwrap_or_default();
                    path.replacen("~", &home, 1)
                } else {
                    path.to_string()
                };

                let _ = Command::new("rm").arg("-rf").arg(&expanded).output();
            }
        }

        // Step 3: Reset connected devices tracking
        #[cfg(target_os = "macos")]
        {
            use crate::mcp::idb_wrapper::CONNECTED_DEVICES;
            let mut devices = CONNECTED_DEVICES.lock().unwrap();
            devices.clear();
            eprintln!("[IdbRecovery] Cleared connected devices tracking");
        }

        // Step 4: Clear any IDB-specific environment variables that might be cached
        unsafe {
            std::env::remove_var("IDB_COMPANION_PORT");
            std::env::remove_var("IDB_GRPC_PORT");
        }

        // Step 5: If using embedded IDB, re-initialize it
        #[cfg(target_os = "macos")]
        {
            use crate::mcp::idb_wrapper::IdbWrapper;
            eprintln!("[IdbRecovery] Re-initializing IDB wrapper...");

            // Force re-initialization by clearing the static path
            if let Err(e) = IdbWrapper::initialize_with_preference(false) {
                eprintln!(
                    "[IdbRecovery] Warning: Failed to re-initialize IDB wrapper: {}",
                    e
                );
            }
        }

        // Update last recovery time
        *last_recovery = Some(Instant::now());

        // Wait a bit for things to settle
        tokio::time::sleep(Duration::from_secs(2)).await;

        eprintln!("[IdbRecovery] Recovery process completed");
        Ok(())
    }

    /// Force disconnect and reconnect for a specific device
    pub async fn force_reconnect_device(&self, device_id: &str) -> Result<()> {
        eprintln!("[IdbRecovery] Force reconnecting device {}...", device_id);

        #[cfg(target_os = "macos")]
        {
            use crate::mcp::idb_wrapper::{CONNECTED_DEVICES, IdbWrapper};

            // Step 1: Remove from connected devices tracking
            {
                let mut devices = CONNECTED_DEVICES.lock().unwrap();
                devices.remove(device_id);
                eprintln!("[IdbRecovery] Removed device {} from tracking", device_id);
            }

            // Step 2: Try to explicitly disconnect via IDB using the correct binary path
            eprintln!("[IdbRecovery] Attempting to disconnect device via IDB...");

            // Get the binary path from IdbWrapper
            let binary_path = IdbWrapper::get_binary_path()
                .unwrap_or_else(|_| std::path::PathBuf::from("idb_companion"));

            let disconnect_output = Command::new(&binary_path)
                .args(["disconnect", device_id])
                .output()
                .ok();

            if let Some(output) = disconnect_output {
                if output.status.success() {
                    eprintln!("[IdbRecovery] Successfully disconnected device");
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    eprintln!("[IdbRecovery] Disconnect command failed: {}", stderr);
                }
            }

            // Step 3: Wait a moment
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Step 4: Force a new connection by using IDB's connect command
            eprintln!(
                "[IdbRecovery] Forcing new connection to device {}...",
                device_id
            );

            // Initialize IDB if needed
            IdbWrapper::initialize()?;

            // Try explicit connect command
            let connect_output = Command::new(&binary_path)
                .args(["connect", device_id])
                .output()
                .ok();

            if let Some(output) = connect_output {
                if output.status.success()
                    || String::from_utf8_lossy(&output.stderr).contains("already connected")
                {
                    eprintln!("[IdbRecovery] Successfully connected to device");
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    eprintln!("[IdbRecovery] Connect command failed: {}", stderr);

                    // If direct connect fails, try with explicit port
                    eprintln!("[IdbRecovery] Retrying with explicit port...");
                    let port_connect = Command::new(&binary_path)
                        .args(["connect", device_id, "--companion-port", "10882"])
                        .output()
                        .ok();

                    if let Some(output) = port_connect {
                        if output.status.success() {
                            eprintln!("[IdbRecovery] Connected successfully with explicit port");
                        }
                    }
                }
            }

            // The ensure_connected method in IdbWrapper will handle the reconnection
            // We just need to trigger it by clearing the device from the cache
            eprintln!("[IdbRecovery] Device {} reconnection attempted", device_id);
        }

        Ok(())
    }

    /// Check if a device is responsive via simctl
    pub async fn check_device_responsive(device_id: &str) -> bool {
        let output = Command::new("xcrun")
            .args(["simctl", "spawn", device_id, "launchctl", "list"])
            .output()
            .ok();

        if let Some(output) = output {
            output.status.success()
        } else {
            false
        }
    }

    /// Handle the specific case where IDB companion is running but not connected properly
    pub async fn recover_stuck_companion(&self) -> Result<()> {
        eprintln!("[IdbRecovery] Recovering stuck IDB companion (running but not connected)...");

        #[cfg(target_os = "macos")]
        {
            use crate::mcp::idb_wrapper::{CONNECTED_DEVICES, IdbWrapper};

            // Step 1: Clear all device connections from tracking
            {
                let mut devices = CONNECTED_DEVICES.lock().unwrap();
                let device_count = devices.len();
                devices.clear();
                eprintln!(
                    "[IdbRecovery] Cleared {} devices from connection tracking",
                    device_count
                );
            }

            // Step 2: Get the IDB companion PID(s)
            let pgrep_output = Command::new("pgrep")
                .arg("-f")
                .arg("idb_companion")
                .output()
                .ok();

            if let Some(output) = pgrep_output {
                if output.status.success() {
                    let pids = String::from_utf8_lossy(&output.stdout);
                    eprintln!("[IdbRecovery] Found IDB companion PIDs: {}", pids.trim());

                    // Step 3: Try to send SIGTERM first
                    for pid in pids.lines() {
                        if let Ok(pid_num) = pid.trim().parse::<i32>() {
                            eprintln!("[IdbRecovery] Sending SIGTERM to PID {}", pid_num);
                            let _ = Command::new("kill")
                                .arg("-TERM")
                                .arg(pid_num.to_string())
                                .output();
                        }
                    }

                    // Wait for graceful shutdown
                    tokio::time::sleep(Duration::from_secs(2)).await;

                    // Step 4: Check if still running and force kill if needed
                    if Self::is_companion_running().await {
                        eprintln!("[IdbRecovery] IDB companion still running, force killing...");
                        let _ = Command::new("pkill")
                            .arg("-9")
                            .arg("-f")
                            .arg("idb_companion")
                            .output();
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            }

            // Step 5: Clear any port bindings by killing lsof processes holding the port
            eprintln!("[IdbRecovery] Clearing port bindings...");
            let lsof_output = Command::new("lsof").args(["-ti", ":10882"]).output().ok();

            if let Some(output) = lsof_output {
                if output.status.success() {
                    let pids = String::from_utf8_lossy(&output.stdout);
                    for pid in pids.lines() {
                        if let Ok(pid_num) = pid.trim().parse::<i32>() {
                            eprintln!(
                                "[IdbRecovery] Killing process {} holding port 10882",
                                pid_num
                            );
                            let _ = Command::new("kill")
                                .arg("-9")
                                .arg(pid_num.to_string())
                                .output();
                        }
                    }
                }
            }

            // Step 6: Clear temp files and locks
            let temp_paths = [
                "/tmp/idb_companion.lock",
                "/tmp/idb_companion.sock",
                "/tmp/notifier*",
            ];

            for path in &temp_paths {
                eprintln!("[IdbRecovery] Removing {}", path);
                let _ = Command::new("rm").arg("-f").arg(path).output();
            }

            // Step 7: Force re-initialization of IDB wrapper
            eprintln!("[IdbRecovery] Re-initializing IDB wrapper with embedded binary...");
            if let Err(e) = IdbWrapper::initialize_with_preference(false) {
                eprintln!(
                    "[IdbRecovery] Warning: Failed to re-initialize IDB wrapper: {}",
                    e
                );

                // Try again with system IDB as fallback
                eprintln!("[IdbRecovery] Trying with system IDB as fallback...");
                if let Err(e) = IdbWrapper::initialize_with_preference(true) {
                    return Err(TestError::Mcp(format!(
                        "Failed to initialize IDB after recovery: {}",
                        e
                    )));
                }
            }

            // Step 8: Wait for things to stabilize
            tokio::time::sleep(Duration::from_secs(2)).await;

            eprintln!("[IdbRecovery] Stuck companion recovery completed");
        }

        Ok(())
    }
}

impl Default for IdbRecovery {
    fn default() -> Self {
        Self::new()
    }
}
