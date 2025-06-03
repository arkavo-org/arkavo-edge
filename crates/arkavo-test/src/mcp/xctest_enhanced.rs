use super::device_manager::DeviceManager;
use super::xctest_compiler::XCTestCompiler;
use super::xctest_unix_bridge::{CommandResponse, XCTestUnixBridge};
use crate::{Result, TestError};
use serde::{Deserialize, Serialize};
use std::process::{Child, Command};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwipeCommand {
    pub id: String,
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub duration: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeTextCommand {
    pub id: String,
    pub text: String,
}

pub struct XCTestEnhanced {
    bridge: Arc<Mutex<XCTestUnixBridge>>,
    compiler: XCTestCompiler,
    _device_manager: Arc<DeviceManager>,
    test_process: Arc<Mutex<Option<Child>>>,
}

impl XCTestEnhanced {
    pub async fn new(device_manager: Arc<DeviceManager>) -> Result<Self> {
        let compiler = XCTestCompiler::new()?;
        let mut bridge = XCTestUnixBridge::new();

        // Start the Unix socket server
        bridge.start().await?;

        Ok(Self {
            bridge: Arc::new(Mutex::new(bridge)),
            compiler,
            _device_manager: device_manager,
            test_process: Arc::new(Mutex::new(None)),
        })
    }

    /// Initialize XCTest runner on the simulator
    pub async fn initialize(&self, device_id: &str) -> Result<()> {
        // Step 1: Compile the XCTest bundle if needed
        let bundle_path = self.compiler.get_xctest_bundle()?;

        // Step 2: Install the test bundle to simulator
        self.install_test_bundle(device_id, &bundle_path)?;

        // Step 3: Run the test bundle
        self.run_test_bundle(device_id).await?;

        // Step 4: Connect to the runner
        let mut bridge = self.bridge.lock().await;
        bridge.connect_to_runner().await?;

        Ok(())
    }

    /// Install test bundle to simulator
    fn install_test_bundle(&self, device_id: &str, bundle_path: &std::path::Path) -> Result<()> {
        let output = Command::new("xcrun")
            .args([
                "simctl",
                "install",
                device_id,
                bundle_path.to_str().unwrap(),
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to install test bundle: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to install test bundle: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    /// Run the test bundle on simulator
    async fn run_test_bundle(&self, device_id: &str) -> Result<()> {
        // Get bundle path and app identifier
        let bundle_path = self.compiler.get_xctest_bundle()?;
        let _bundle_name = bundle_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| TestError::Mcp("Invalid bundle path".to_string()))?;

        // Run using xcrun simctl to launch the test runner
        let child = Command::new("xcrun")
            .args([
                "simctl",
                "spawn",
                device_id,
                "xctest",
                bundle_path.to_str().unwrap(),
            ])
            .spawn()
            .map_err(|e| TestError::Mcp(format!("Failed to run test bundle: {}", e)))?;

        // Store the process handle for cleanup
        let mut process_guard = self.test_process.lock().await;
        *process_guard = Some(child);

        // Let it start up
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        Ok(())
    }

    /// Send a tap command
    pub async fn tap(&self, x: f64, y: f64) -> Result<CommandResponse> {
        let command = XCTestUnixBridge::create_coordinate_tap(x, y);
        let bridge = self.bridge.lock().await;
        bridge.send_tap_command(command).await
    }

    /// Send a tap by text
    pub async fn tap_by_text(&self, text: &str, timeout: Option<f64>) -> Result<CommandResponse> {
        let command = XCTestUnixBridge::create_text_tap(text.to_string(), timeout);
        let bridge = self.bridge.lock().await;
        bridge.send_tap_command(command).await
    }

    /// Send a tap by accessibility ID
    pub async fn tap_by_accessibility_id(
        &self,
        id: &str,
        timeout: Option<f64>,
    ) -> Result<CommandResponse> {
        let command = XCTestUnixBridge::create_accessibility_tap(id.to_string(), timeout);
        let bridge = self.bridge.lock().await;
        bridge.send_tap_command(command).await
    }

    /// Check if XCTest is available and connected
    pub async fn is_available(&self) -> bool {
        let bridge = self.bridge.lock().await;
        bridge.is_connected()
    }

    /// Cleanup test process
    pub async fn cleanup(&self) -> Result<()> {
        let mut process_guard = self.test_process.lock().await;
        if let Some(mut child) = process_guard.take() {
            // Try to terminate gracefully first
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    }
}

impl Drop for XCTestEnhanced {
    fn drop(&mut self) {
        // Best effort cleanup - can't be async in Drop
        if let Ok(mut guard) = self.test_process.try_lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
            }
        }
    }
}

/// Alternative implementation using simctl for basic interactions
pub struct SimctlInteraction;

impl SimctlInteraction {
    /// Send touch event using simctl (iOS 14+)
    pub fn send_touch(device_id: &str, x: f64, y: f64) -> Result<()> {
        // Try using simctl io for touch events (available in newer Xcode versions)
        let output = Command::new("xcrun")
            .args([
                "simctl",
                "io",
                device_id,
                "touch",
                &format!("{},{}", x as i32, y as i32),
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to send touch: {}", e)))?;

        if !output.status.success() {
            // Fallback to using Accessibility
            return Err(TestError::Mcp(
                "simctl touch not available, XCTest required".to_string(),
            ));
        }

        Ok(())
    }

    /// Send swipe using simctl
    pub fn send_swipe(
        device_id: &str,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        duration: f64,
    ) -> Result<()> {
        // Try using simctl io for swipe events
        let output = Command::new("xcrun")
            .args([
                "simctl",
                "io",
                device_id,
                "swipe",
                &format!("{},{}", x1 as i32, y1 as i32),
                &format!("{},{}", x2 as i32, y2 as i32),
                &format!("{}", duration),
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to send swipe: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(
                "simctl swipe not available, XCTest required".to_string(),
            ));
        }

        Ok(())
    }
}
