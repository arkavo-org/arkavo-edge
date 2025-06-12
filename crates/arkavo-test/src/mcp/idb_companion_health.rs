use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::Result;

/// Tracks health metrics for IDB companion instances
#[derive(Debug, Clone)]
pub struct CompanionHealthMetrics {
    pub device_id: String,
    pub last_successful_tap: Option<Instant>,
    pub last_failed_tap: Option<Instant>,
    pub consecutive_failures: u32,
    pub total_taps_attempted: u64,
    pub total_taps_succeeded: u64,
    pub average_tap_latency_ms: f64,
    pub last_health_check: Instant,
    pub companion_pid: Option<u32>,
    pub is_responsive: bool,
    pub connection_established: bool,
}

impl CompanionHealthMetrics {
    fn new(device_id: String) -> Self {
        Self {
            device_id,
            last_successful_tap: None,
            last_failed_tap: None,
            consecutive_failures: 0,
            total_taps_attempted: 0,
            total_taps_succeeded: 0,
            average_tap_latency_ms: 0.0,
            last_health_check: Instant::now(),
            companion_pid: None,
            is_responsive: false,
            connection_established: false,
        }
    }
    
    fn success_rate(&self) -> f64 {
        if self.total_taps_attempted == 0 {
            0.0
        } else {
            (self.total_taps_succeeded as f64 / self.total_taps_attempted as f64) * 100.0
        }
    }
}

/// Global health tracking for all IDB companions
static COMPANION_HEALTH: Lazy<Mutex<HashMap<String, CompanionHealthMetrics>>> = 
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Manages IDB companion health and recovery
pub struct IdbCompanionHealth;

impl IdbCompanionHealth {
    /// Check if IDB companion is healthy for a device
    pub async fn check_health(device_id: &str) -> Result<bool> {
        eprintln!("[IdbCompanionHealth] Checking health for device {}", device_id);
        
        // Update or create metrics
        let mut health_map = COMPANION_HEALTH.lock().unwrap();
        let metrics = health_map.entry(device_id.to_string())
            .or_insert_with(|| CompanionHealthMetrics::new(device_id.to_string()));
        
        // Check various health indicators
        let mut is_healthy = true;
        
        // 1. Check if companion process is running
        if let Some(pid) = metrics.companion_pid {
            if !Self::is_process_running(pid) {
                eprintln!("[IdbCompanionHealth] Companion process {} is not running", pid);
                metrics.companion_pid = None;
                is_healthy = false;
            }
        }
        
        // 2. Check if we can list targets (basic connectivity test)
        if is_healthy {
            match Self::test_companion_connection(device_id).await {
                Ok(connected) => {
                    metrics.connection_established = connected;
                    if !connected {
                        eprintln!("[IdbCompanionHealth] Companion not connected to device");
                        is_healthy = false;
                    }
                }
                Err(e) => {
                    eprintln!("[IdbCompanionHealth] Connection test failed: {}", e);
                    metrics.connection_established = false;
                    is_healthy = false;
                }
            }
        }
        
        // 3. Check failure rate
        if metrics.consecutive_failures > 5 {
            eprintln!("[IdbCompanionHealth] Too many consecutive failures: {}", metrics.consecutive_failures);
            is_healthy = false;
        }
        
        // 4. Check if companion is responsive (hasn't been used successfully in a while)
        if let Some(last_success) = metrics.last_successful_tap {
            if last_success.elapsed() > Duration::from_secs(300) { // 5 minutes
                eprintln!("[IdbCompanionHealth] No successful taps in last 5 minutes");
                metrics.is_responsive = false;
            }
        }
        
        metrics.last_health_check = Instant::now();
        metrics.is_responsive = is_healthy;
        
        eprintln!("[IdbCompanionHealth] Health check result: {} (success rate: {:.1}%)", 
            if is_healthy { "HEALTHY" } else { "UNHEALTHY" },
            metrics.success_rate()
        );
        
        Ok(is_healthy)
    }
    
    /// Record a tap attempt result
    pub fn record_tap_result(device_id: &str, success: bool, latency_ms: u64) {
        let mut health_map = COMPANION_HEALTH.lock().unwrap();
        let metrics = health_map.entry(device_id.to_string())
            .or_insert_with(|| CompanionHealthMetrics::new(device_id.to_string()));
        
        metrics.total_taps_attempted += 1;
        
        if success {
            metrics.total_taps_succeeded += 1;
            metrics.last_successful_tap = Some(Instant::now());
            metrics.consecutive_failures = 0;
            
            // Update average latency
            let new_avg = if metrics.total_taps_succeeded == 1 {
                latency_ms as f64
            } else {
                let prev_total = metrics.average_tap_latency_ms * (metrics.total_taps_succeeded - 1) as f64;
                (prev_total + latency_ms as f64) / metrics.total_taps_succeeded as f64
            };
            metrics.average_tap_latency_ms = new_avg;
        } else {
            metrics.last_failed_tap = Some(Instant::now());
            metrics.consecutive_failures += 1;
        }
        
        eprintln!("[IdbCompanionHealth] Tap {} for device {} (latency: {}ms, success rate: {:.1}%)",
            if success { "succeeded" } else { "failed" },
            device_id,
            latency_ms,
            metrics.success_rate()
        );
    }
    
    /// Get health metrics for a device
    pub fn get_metrics(device_id: &str) -> Option<CompanionHealthMetrics> {
        let health_map = COMPANION_HEALTH.lock().unwrap();
        health_map.get(device_id).cloned()
    }
    
    /// Reset health metrics for a device (useful after recovery)
    pub fn reset_metrics(device_id: &str) {
        let mut health_map = COMPANION_HEALTH.lock().unwrap();
        if let Some(metrics) = health_map.get_mut(device_id) {
            metrics.consecutive_failures = 0;
            metrics.last_failed_tap = None;
            eprintln!("[IdbCompanionHealth] Reset metrics for device {}", device_id);
        }
    }
    
    /// Check if a process is running by PID
    #[cfg(target_os = "macos")]
    fn is_process_running(pid: u32) -> bool {
        let output = Command::new("ps")
            .args(["-p", &pid.to_string()])
            .output()
            .ok();
            
        if let Some(output) = output {
            output.status.success() && String::from_utf8_lossy(&output.stdout).lines().count() > 1
        } else {
            false
        }
    }
    
    #[cfg(not(target_os = "macos"))]
    fn is_process_running(_pid: u32) -> bool {
        false
    }
    
    /// Test basic companion connectivity
    async fn test_companion_connection(device_id: &str) -> Result<bool> {
        #[cfg(target_os = "macos")]
        {
            use super::idb_wrapper::IdbWrapper;
            
            // Try to list targets
            match IdbWrapper::list_targets().await {
                Ok(targets) => {
                    // Check if our device is in the list
                    if let Some(target_array) = targets.as_array() {
                        let device_found = target_array.iter().any(|t| {
                            t.get("udid").and_then(|u| u.as_str()) == Some(device_id)
                        });
                        Ok(device_found)
                    } else {
                        Ok(false)
                    }
                }
                Err(_) => Ok(false)
            }
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            Ok(false)
        }
    }
    
    /// Perform recovery for unhealthy companion
    pub async fn recover_companion(device_id: &str) -> Result<()> {
        eprintln!("[IdbCompanionHealth] Starting recovery for device {}", device_id);
        
        #[cfg(target_os = "macos")]
        {
            use super::idb_recovery::IdbRecovery;
            
            // 1. Kill any stuck companion processes
            let recovery = IdbRecovery::new();
            let _ = recovery.kill_stuck_processes().await;
            
            // 2. Clear connection state
            {
                let mut health_map = COMPANION_HEALTH.lock().unwrap();
                if let Some(metrics) = health_map.get_mut(device_id) {
                    metrics.companion_pid = None;
                    metrics.connection_established = false;
                }
            }
            
            // 3. Wait a bit for cleanup
            tokio::time::sleep(Duration::from_secs(2)).await;
            
            // 4. Re-initialize IDB
            use super::idb_wrapper::IdbWrapper;
            IdbWrapper::initialize()?;
            
            // 5. Ensure companion is running for this device
            IdbWrapper::ensure_companion_running(device_id).await?;
            
            // 6. Reset health metrics
            Self::reset_metrics(device_id);
            
            eprintln!("[IdbCompanionHealth] Recovery completed for device {}", device_id);
        }
        
        Ok(())
    }
    
    /// Get overall system health report
    pub fn get_health_report() -> serde_json::Value {
        let health_map = COMPANION_HEALTH.lock().unwrap();
        let mut report = serde_json::Map::new();
        
        for (device_id, metrics) in health_map.iter() {
            report.insert(device_id.clone(), serde_json::json!({
                "success_rate": format!("{:.1}%", metrics.success_rate()),
                "consecutive_failures": metrics.consecutive_failures,
                "total_attempts": metrics.total_taps_attempted,
                "average_latency_ms": format!("{:.1}", metrics.average_tap_latency_ms),
                "is_healthy": metrics.is_responsive,
                "connected": metrics.connection_established,
                "last_check": format!("{:?} ago", metrics.last_health_check.elapsed()),
            }));
        }
        
        serde_json::Value::Object(report)
    }
}