use super::*;
use crate::mcp::calibration::agent::CalibrationAgentImpl;
use crate::mcp::calibration::data::CalibrationDataStore;
use crate::mcp::calibration::reference_app::ReferenceAppInterface;
use crate::mcp::calibration::verification::{Coordinate, VerificationReader};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
#[cfg(target_os = "macos")]
use crate::mcp::idb_wrapper::IdbWrapper;
#[cfg(target_os = "macos")]
use crate::mcp::idb_recovery::IdbRecovery;

pub struct CalibrationServer {
    pub data_store: Arc<CalibrationDataStore>,
    active_calibrations: Arc<RwLock<HashMap<String, CalibrationSession>>>,
    auto_monitor: Arc<RwLock<AutoMonitor>>,
    #[cfg(target_os = "macos")]
    idb_recovery: Arc<IdbRecovery>,
}

struct CalibrationSession {
    device_id: String,
    reference_app: ReferenceAppInterface,
    start_time: chrono::DateTime<chrono::Utc>,
    status: CalibrationStatus,
    idb_status: IdbStatus,
    last_tap_time: Option<chrono::DateTime<chrono::Utc>>,
    tap_count: u32,
}

#[derive(Debug, Clone)]
enum CalibrationStatus {
    Initializing,
    Validating,
    Complete,
    Failed(String),
}

struct AutoMonitor {
    enabled: bool,
    check_interval_hours: u64,
    recalibration_threshold_hours: u64,
}

impl CalibrationServer {
    async fn check_idb_health(&self, session_id: &str, device_id: &str) -> Result<bool, CalibrationError> {
        #[cfg(target_os = "macos")]
        {
            // Initialize IDB if not already done
            eprintln!("[CalibrationServer::check_idb_health] Initializing IDB wrapper...");
            if let Err(e) = IdbWrapper::initialize() {
                eprintln!("[CalibrationServer::check_idb_health] IDB initialization failed: {}", e);
                self.update_idb_status(session_id, IdbStatus {
                    connected: false,
                    last_health_check: Some(chrono::Utc::now()),
                    last_error: Some(format!("IDB initialization failed: {}", e)),
                    companion_running: false,
                }).await;
                return Ok(false);
            }
            eprintln!("[CalibrationServer::check_idb_health] IDB wrapper initialized successfully");
            
            // Ensure companion is running for this specific device
            eprintln!("[CalibrationServer::check_idb_health] Ensuring IDB companion is running for device {}...", device_id);
            match IdbWrapper::ensure_companion_running(device_id).await {
                Ok(_) => {
                    eprintln!("[CalibrationServer::check_idb_health] IDB companion started/verified for device {}", device_id);
                }
                Err(e) => {
                    eprintln!("[CalibrationServer::check_idb_health] Failed to ensure companion running: {}", e);
                    eprintln!("[CalibrationServer::check_idb_health] Error details: {:?}", e);
                    self.update_idb_status(session_id, IdbStatus {
                        connected: false,
                        last_health_check: Some(chrono::Utc::now()),
                        last_error: Some(format!("Failed to start IDB companion: {}", e)),
                        companion_running: false,
                    }).await;
                    return Ok(false);
                }
            }
            
            // Check if IDB companion process is running and port is accessible
            eprintln!("[CalibrationServer::check_idb_health] Checking companion process status...");
            let companion_running = IdbRecovery::is_companion_running().await;
            eprintln!("[CalibrationServer::check_idb_health] Companion process running: {}", companion_running);
            
            eprintln!("[CalibrationServer::check_idb_health] Checking port 10882 accessibility...");
            let port_accessible = IdbRecovery::is_companion_port_accessible().await;
            eprintln!("[CalibrationServer::check_idb_health] Port 10882 accessible: {}", port_accessible);
            
            // If companion is running but port not accessible, it's stuck
            if companion_running && !port_accessible {
                eprintln!("[CalibrationServer::check_idb_health] DETECTED: Companion is running but port is not accessible - process is stuck");
                self.update_idb_status(session_id, IdbStatus {
                    connected: false,
                    last_health_check: Some(chrono::Utc::now()),
                    last_error: Some("IDB companion running but not accepting connections".to_string()),
                    companion_running: true,
                }).await;
                
                // Use the specific recovery method for stuck companion
                self.idb_recovery.recover_stuck_companion().await
                    .map_err(|e| CalibrationError::InteractionFailed(format!("Failed to recover stuck IDB: {}", e)))?;
                
                // Wait for recovery to take effect
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                
                // Re-check after recovery
                let _companion_running = IdbRecovery::is_companion_running().await;
                let _port_accessible = IdbRecovery::is_companion_port_accessible().await;
            }
            
            // Try a simple IDB command to check if it's working
            eprintln!("[CalibrationServer::check_idb_health] Testing IDB connection with list_targets command...");
            match IdbWrapper::list_targets().await {
                Ok(targets) => {
                    eprintln!("[CalibrationServer::check_idb_health] list_targets succeeded, checking for device {}...", device_id);
                    let device_found = targets.as_array()
                        .map(|arr| {
                            eprintln!("[CalibrationServer::check_idb_health] Found {} targets", arr.len());
                            arr.iter().any(|t| {
                                let udid = t.get("udid").and_then(|u| u.as_str()).unwrap_or("unknown");
                                let matches = udid == device_id;
                                if matches {
                                    eprintln!("[CalibrationServer::check_idb_health] Found matching device: {}", udid);
                                }
                                matches
                            })
                        })
                        .unwrap_or(false);
                    eprintln!("[CalibrationServer::check_idb_health] Device {} found in targets: {}", device_id, device_found);
                    
                    // Check if IDB is actually connected to this specific device
                    // Just having companion running doesn't mean it's connected to our device
                    let is_connected = if device_found && companion_running {
                        // Try a simple IDB command to verify connection
                        eprintln!("[CalibrationServer::check_idb_health] Device found and companion running, verifying connection with list_apps...");
                        match IdbWrapper::list_apps(device_id).await {
                            Ok(_) => {
                                eprintln!("[CalibrationServer::check_idb_health] list_apps succeeded - IDB is fully connected to device");
                                true
                            }
                            Err(e) => {
                                eprintln!("[CalibrationServer::check_idb_health] list_apps failed: {}", e);
                                // Check if this is a framework loading error
                                let error_str = e.to_string();
                                if error_str.contains("Library not loaded") || error_str.contains("FBControlCore") {
                                    eprintln!("[CalibrationServer::check_idb_health] Framework loading error detected");
                                }
                                false
                            }
                        }
                    } else {
                        eprintln!("[CalibrationServer::check_idb_health] Device not found ({}) or companion not running ({})", device_found, companion_running);
                        false
                    };
                    
                    // If device is found but companion not running or not fully connected, try to ensure connection
                    if device_found && (!companion_running || !is_connected) {
                        eprintln!("[CalibrationServer::check_idb_health] Device found but connection issue detected");
                        
                        // Provide helpful error message
                        if !companion_running {
                            eprintln!("[CalibrationServer::check_idb_health] IDB companion is not running");
                        } else if !is_connected {
                            eprintln!("[CalibrationServer::check_idb_health] IDB companion is running but not connected to device");
                        }
                        
                        // If companion is running but not connected, use the stuck recovery
                        if companion_running && !is_connected {
                            eprintln!("[CalibrationServer::check_idb_health] Attempting stuck companion recovery...");
                            match self.idb_recovery.recover_stuck_companion().await {
                                Ok(_) => eprintln!("[CalibrationServer::check_idb_health] Stuck companion recovery completed"),
                                Err(e) => eprintln!("[CalibrationServer::check_idb_health] Stuck companion recovery failed: {}", e),
                            }
                        } else {
                            // Otherwise just try to reconnect the device
                            eprintln!("[CalibrationServer::check_idb_health] Attempting to force reconnect device...");
                            match self.idb_recovery.force_reconnect_device(device_id).await {
                                Ok(_) => eprintln!("[CalibrationServer::check_idb_health] Device reconnection completed"),
                                Err(e) => eprintln!("[CalibrationServer::check_idb_health] Device reconnection failed: {}", e),
                            }
                        }
                    }
                    
                    let final_status = IdbStatus {
                        connected: is_connected,
                        last_health_check: Some(chrono::Utc::now()),
                        last_error: if is_connected { 
                            None 
                        } else if !device_found {
                            Some(format!("Device {} not found in IDB targets list", device_id))
                        } else if !companion_running {
                            Some("IDB companion process is not running".to_string())
                        } else {
                            Some("Device found but IDB not fully connected. Connection verification failed.".to_string())
                        },
                        companion_running,
                    };
                    
                    eprintln!("[CalibrationServer::check_idb_health] Final IDB status: connected={}, companion_running={}, error={:?}", 
                        final_status.connected, 
                        final_status.companion_running, 
                        final_status.last_error
                    );
                    
                    self.update_idb_status(session_id, final_status).await;
                    
                    eprintln!("[CalibrationServer::check_idb_health] Returning health status: {}", is_connected);
                    Ok(is_connected)
                }
                Err(e) => {
                    eprintln!("[CalibrationServer::check_idb_health] list_targets failed: {}", e);
                    // Check if it's a connection issue
                    let error_str = e.to_string();
                    let is_connection_issue = error_str.contains("Connection refused") || 
                                            error_str.contains("failed to connect");
                    
                    if is_connection_issue {
                        eprintln!("[CalibrationServer::check_idb_health] Connection issue detected - companion may not be running or port blocked");
                    }
                    
                    self.update_idb_status(session_id, IdbStatus {
                        connected: false,
                        last_health_check: Some(chrono::Utc::now()),
                        last_error: Some(format!("IDB health check failed: {}", e)),
                        companion_running: companion_running && !is_connection_issue,
                    }).await;
                    
                    eprintln!("[CalibrationServer::check_idb_health] Returning health status: false (list_targets failed)");
                    Ok(false)
                }
            }
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            Ok(false)
        }
    }
    
    async fn update_idb_status(&self, session_id: &str, status: IdbStatus) {
        let mut sessions = self.active_calibrations.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            session.idb_status = status;
        }
    }
    
    async fn record_tap(&self, session_id: &str) {
        let mut sessions = self.active_calibrations.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            session.last_tap_time = Some(chrono::Utc::now());
            session.tap_count += 1;
        }
    }
    pub fn new(storage_path: PathBuf) -> Result<Self, CalibrationError> {
        Ok(Self {
            data_store: Arc::new(CalibrationDataStore::new(storage_path)?),
            active_calibrations: Arc::new(RwLock::new(HashMap::new())),
            auto_monitor: Arc::new(RwLock::new(AutoMonitor {
                enabled: true,
                check_interval_hours: 24,
                recalibration_threshold_hours: 24 * 7, // 1 week
            })),
            #[cfg(target_os = "macos")]
            idb_recovery: Arc::new(IdbRecovery::new()),
        })
    }
    
    pub async fn start_calibration(
        &self,
        device_id: String,
        reference_bundle_id: Option<String>,
    ) -> Result<String, CalibrationError> {
        // Check if calibration already in progress
        {
            let sessions = self.active_calibrations.read().await;
            if sessions.contains_key(&device_id) {
                return Err(CalibrationError::ConfigurationError(
                    "Calibration already in progress for this device".to_string()
                ));
            }
        }
        
        // Create new session
        let session_id = format!("cal_{}_{}", device_id, chrono::Utc::now().timestamp());
        
        let mut reference_app = ReferenceAppInterface::new(device_id.clone());
        if let Some(bundle_id) = reference_bundle_id {
            reference_app = reference_app.with_bundle_id(bundle_id);
        }
        
        let session = CalibrationSession {
            device_id: device_id.clone(),
            reference_app,
            start_time: chrono::Utc::now(),
            status: CalibrationStatus::Initializing,
            idb_status: IdbStatus {
                connected: false,
                last_health_check: None,
                last_error: None,
                companion_running: false,
            },
            last_tap_time: None,
            tap_count: 0,
        };
        
        {
            let mut sessions = self.active_calibrations.write().await;
            sessions.insert(session_id.clone(), session);
        }
        
        // Start calibration in background
        let server = self.clone();
        let session_id_clone = session_id.clone();
        tokio::spawn(async move {
            eprintln!("Calibration: Background task started for session {}", session_id_clone);
            match server.run_calibration(&session_id_clone).await {
                Ok(_) => {
                    eprintln!("Calibration: Completed successfully for session {}", session_id_clone);
                }
                Err(e) => {
                    eprintln!("Calibration: Failed with error: {}", e);
                    server.mark_session_failed(&session_id_clone, e.to_string()).await;
                }
            }
        });
        
        Ok(session_id)
    }
    
    async fn run_calibration(&self, session_id: &str) -> Result<(), CalibrationError> {
        eprintln!("Calibration: run_calibration started for session {}", session_id);
        let start_time = std::time::Instant::now();
        const CALIBRATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
        
        // Get session
        eprintln!("Calibration: Getting session data...");
        let (device_id, agent, reference_app) = {
            let sessions = self.active_calibrations.read().await;
            let session = sessions.get(session_id)
                .ok_or_else(|| CalibrationError::ConfigurationError("Session not found".to_string()))?;
            
            eprintln!("Calibration: Found session for device {}", session.device_id);
            
            let agent = CalibrationAgentImpl::new(session.device_id.clone())?;
            eprintln!("Calibration: Created agent");
            
            (session.device_id.clone(), 
             agent,
             session.reference_app.clone())
        };
        eprintln!("Calibration: Session data retrieved successfully");
        
        // Phase 1: Initialize
        self.update_session_status(session_id, CalibrationStatus::Initializing).await;
        eprintln!("Calibration: Starting calibration process for device {}", device_id);
        eprintln!("Calibration: Expected duration: 20-30 seconds");
        
        // Check IDB health before starting
        eprintln!("Calibration: Checking IDB companion status...");
        let idb_healthy = self.check_idb_health(session_id, &device_id).await?;
        if !idb_healthy {
            // Get initial IDB status for diagnostics
            let initial_status = {
                let sessions = self.active_calibrations.read().await;
                sessions.get(session_id)
                    .map(|s| s.idb_status.clone())
            };
            
            if let Some(status) = initial_status {
                eprintln!("Calibration: WARNING - IDB companion not properly connected");
                eprintln!("  - Companion running: {}", status.companion_running);
                eprintln!("  - Connected: {}", status.connected);
                if let Some(error) = &status.last_error {
                    eprintln!("  - Last error: {}", error);
                }
            }
            
            eprintln!("Calibration: Attempting IDB recovery...");
            #[cfg(target_os = "macos")]
            {
                self.idb_recovery.attempt_recovery().await
                    .map_err(|e| CalibrationError::InteractionFailed(format!("IDB recovery failed: {}", e)))?;
                // Wait a bit for recovery to take effect
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                
                // Check again with detailed diagnostics
                eprintln!("Calibration: Re-checking IDB health after recovery...");
                let idb_healthy_after = self.check_idb_health(session_id, &device_id).await?;
                if !idb_healthy_after {
                    // Get detailed IDB status for error message
                    let idb_status = {
                        let sessions = self.active_calibrations.read().await;
                        sessions.get(session_id)
                            .map(|s| s.idb_status.clone())
                            .unwrap_or(IdbStatus {
                                connected: false,
                                last_health_check: Some(chrono::Utc::now()),
                                last_error: Some("Unknown status".to_string()),
                                companion_running: false,
                            })
                    };
                    
                    let mut error_details = vec![
                        "IDB connection could not be established after recovery".to_string(),
                    ];
                    
                    if let Some(last_error) = &idb_status.last_error {
                        error_details.push(format!("Last error: {}", last_error));
                    }
                    
                    error_details.push(format!("Companion running: {}", idb_status.companion_running));
                    error_details.push(format!("Connected: {}", idb_status.connected));
                    
                    // Check specific conditions
                    if idb_status.companion_running && !idb_status.connected {
                        error_details.push("IDB companion process is running but not connected to device".to_string());
                        error_details.push("This usually indicates a port binding issue or device communication problem".to_string());
                    } else if !idb_status.companion_running {
                        error_details.push("IDB companion process failed to start".to_string());
                        error_details.push("Check if the embedded binary is properly extracted and executable".to_string());
                    }
                    
                    // Add recovery suggestions
                    error_details.push("\nPossible solutions:".to_string());
                    error_details.push("1. Kill any existing idb_companion processes: pkill -f idb_companion".to_string());
                    error_details.push("2. Restart the simulator".to_string());
                    error_details.push("3. Check if port 10882 is available: lsof -i :10882".to_string());
                    error_details.push("4. Try using system IDB: export ARKAVO_USE_SYSTEM_IDB=1".to_string());
                    
                    let full_error = error_details.join("\n");
                    eprintln!("Calibration: CRITICAL ERROR - {}", full_error);
                    
                    return Err(CalibrationError::InteractionFailed(full_error));
                }
                eprintln!("Calibration: IDB recovery successful, connection established");
            }
        }
        
        // Launch the reference app (this will launch in calibration mode if available)
        eprintln!("Calibration: Launching reference app...");
        reference_app.launch()?;
        eprintln!("Calibration: Reference app launch command completed");
        
        // Phase 2: Discovery
        eprintln!("Calibration: Getting device parameters...");
        let device_params = agent.get_device_parameters()?;
        eprintln!("Calibration: Device parameters retrieved: {} ({}x{})", 
            device_params.device_name, 
            device_params.screen_resolution.width,
            device_params.screen_resolution.height
        );
        
        // Phase 3: Check if reference app launched successfully
        eprintln!("Calibration: Checking if reference app is available...");
        let has_reference_app = reference_app.is_available();
        if !has_reference_app {
            // This is a critical error - calibration REQUIRES the reference app
            return Err(CalibrationError::ConfigurationError(
                "CRITICAL: ArkavoReference app is not installed. This app is required for calibration.\n\
                The calibration system cannot function without visual feedback from the reference app.\n\
                Please use 'calibration_manager' with action 'install_reference_app' to install it first.".to_string()
            ));
        }
        
        eprintln!("Calibration: Reference app launched successfully in calibration mode");
        eprintln!("Calibration: Waiting for calibration view to fully load...");
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
        eprintln!("Calibration: Wait completed, updating status to validating...");
        
        // Update status to validating now that app is ready
        self.update_session_status(session_id, CalibrationStatus::Validating).await;
        eprintln!("Calibration: Status updated to VALIDATING");
        eprintln!("Calibration: Phase 2 - Beginning tap sequence and verification");
        
        // Phase 4: Run calibration with verification
        let verification_reader = VerificationReader::new(device_id.clone());
        let mut coordinate_offset = Coordinate { x: 0.0, y: 0.0 };
        let mut calibration_attempts = 0;
        const MAX_ATTEMPTS: u32 = 3;
        
        loop {
            // Check timeout
            if start_time.elapsed() > CALIBRATION_TIMEOUT {
                eprintln!("Calibration: Timeout after {} seconds", CALIBRATION_TIMEOUT.as_secs());
                return Err(CalibrationError::InteractionFailed(
                    format!("Calibration timed out after {} seconds", CALIBRATION_TIMEOUT.as_secs())
                ));
            }
            
            calibration_attempts += 1;
            if calibration_attempts > MAX_ATTEMPTS {
                eprintln!("Calibration: Max attempts reached, using best offset found");
                break;
            }
            
            eprintln!("Calibration: Attempt {} of {}", calibration_attempts, MAX_ATTEMPTS);
            
            // Clear previous results if any
            let _ = verification_reader.read_calibration_results();
            
            // Run calibration taps using percentage-based coordinates
            // Get screen dimensions from device parameters
            let screen_width = device_params.screen_resolution.width;
            let screen_height = device_params.screen_resolution.height;
            
            let test_points = [
                (screen_width * 0.2, screen_height * 0.2),   // 20%, 20%
                (screen_width * 0.8, screen_height * 0.2),   // 80%, 20%
                (screen_width * 0.5, screen_height * 0.5),   // 50%, 50%
                (screen_width * 0.2, screen_height * 0.8),   // 20%, 80%
                (screen_width * 0.8, screen_height * 0.8),   // 80%, 80%
            ];
            
            eprintln!("Calibration: Starting tap sequence...");
            
            // Track time since last successful tap for watchdog
            let mut last_successful_tap = std::time::Instant::now();
            let mut stuck_recovery_attempted = false;
            
            for (idx, (x, y)) in test_points.iter().enumerate() {
                eprintln!("Calibration: Tapping point {}/{}: ({}, {})", idx + 1, test_points.len(), x, y);
                
                // Watchdog: Check if we're stuck (no taps for 15 seconds)
                if last_successful_tap.elapsed() > std::time::Duration::from_secs(15) && !stuck_recovery_attempted {
                    eprintln!("Calibration: WATCHDOG - No successful taps for 15 seconds, attempting auto-recovery");
                    
                    #[cfg(target_os = "macos")]
                    {
                        // Check if companion is running but stuck
                        let companion_running = IdbRecovery::is_companion_running().await;
                        let port_accessible = IdbRecovery::is_companion_port_accessible().await;
                        
                        eprintln!("Calibration: WATCHDOG - Companion running: {}, Port accessible: {}", 
                            companion_running, port_accessible);
                        
                        // Use appropriate recovery method
                        if companion_running && !port_accessible {
                            eprintln!("Calibration: WATCHDOG - Using targeted stuck companion recovery...");
                            if let Ok(_) = self.idb_recovery.recover_stuck_companion().await {
                                eprintln!("Calibration: WATCHDOG - Stuck companion recovery completed");
                            }
                        } else {
                            eprintln!("Calibration: WATCHDOG - Using general IDB recovery...");
                            if let Ok(_) = self.idb_recovery.attempt_recovery().await {
                                eprintln!("Calibration: WATCHDOG - General recovery completed");
                            }
                        }
                        
                        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                        
                        // Re-check IDB health
                        let idb_ok = self.check_idb_health(session_id, &device_id).await?;
                        eprintln!("Calibration: IDB health after recovery: {}", if idb_ok { "healthy" } else { "unhealthy" });
                        stuck_recovery_attempted = true;
                    }
                }
                
                // Apply current offset correction
                let corrected_x = x - coordinate_offset.x;
                let corrected_y = y - coordinate_offset.y;
                
                // Check IDB health periodically during taps
                if idx % 2 == 0 {
                    let idb_ok = self.check_idb_health(session_id, &device_id).await?;
                    if !idb_ok {
                        eprintln!("Calibration: IDB health check failed during tap sequence");
                    }
                }
                
                // Wrap tap execution with a timeout using spawn_blocking since execute_tap is synchronous
                let tap_timeout = std::time::Duration::from_secs(10);
                let agent_clone = agent.clone();
                
                let tap_result = tokio::time::timeout(
                    tap_timeout,
                    tokio::task::spawn_blocking(move || {
                        agent_clone.execute_tap(corrected_x, corrected_y)
                    })
                ).await;
                
                match tap_result {
                    Ok(Ok(Ok(_))) => {
                        eprintln!("Calibration: Tap {} executed successfully", idx + 1);
                        self.record_tap(session_id).await;
                        last_successful_tap = std::time::Instant::now();
                        stuck_recovery_attempted = false; // Reset recovery flag on success
                    },
                    Ok(Ok(Err(e))) => {
                        eprintln!("Calibration: Warning - Tap {} failed: {}", idx + 1, e);
                        
                        // Check if this is an IDB-related failure
                        if e.to_string().contains("idb_companion") {
                            eprintln!("Calibration: IDB failure detected, attempting recovery...");
                            
                            #[cfg(target_os = "macos")]
                            {
                                // Check the specific IDB state
                                let companion_running = IdbRecovery::is_companion_running().await;
                                let port_accessible = IdbRecovery::is_companion_port_accessible().await;
                                
                                // Use appropriate recovery
                                let recovery_success = if companion_running && !port_accessible {
                                    eprintln!("Calibration: Using targeted stuck companion recovery...");
                                    self.idb_recovery.recover_stuck_companion().await.is_ok()
                                } else {
                                    eprintln!("Calibration: Using general IDB recovery...");
                                    self.idb_recovery.attempt_recovery().await.is_ok()
                                };
                                
                                if recovery_success {
                                    eprintln!("Calibration: IDB recovery completed, retrying tap...");
                                    
                                    // Wait a bit for recovery to take effect
                                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                                    
                                    // Retry the tap once
                                    let agent_retry = agent.clone();
                                    let retry_result = tokio::task::spawn_blocking(move || {
                                        agent_retry.execute_tap(corrected_x, corrected_y)
                                    }).await;
                                    
                                    if let Ok(Ok(_)) = retry_result {
                                        eprintln!("Calibration: Retry tap {} succeeded after recovery", idx + 1);
                                        self.record_tap(session_id).await;
                                        last_successful_tap = std::time::Instant::now();
                                        continue;
                                    } else {
                                        eprintln!("Calibration: Retry tap {} failed after recovery", idx + 1);
                                    }
                                }
                            }
                        }
                        
                        // Update IDB error status
                        self.update_idb_status(session_id, IdbStatus {
                            connected: false,
                            last_health_check: Some(chrono::Utc::now()),
                            last_error: Some(format!("Tap failed: {}", e)),
                            companion_running: false,
                        }).await;
                    },
                    Ok(Err(_)) => {
                        eprintln!("Calibration: Tap {} - spawn_blocking task failed", idx + 1);
                    },
                    Err(_) => {
                        eprintln!("Calibration: Tap {} timed out after 10 seconds", idx + 1);
                        
                        // Timeout indicates stuck IDB - attempt recovery
                        #[cfg(target_os = "macos")]
                        {
                            eprintln!("Calibration: Tap timeout detected, checking IDB state...");
                            
                            // Check the specific IDB state
                            let companion_running = IdbRecovery::is_companion_running().await;
                            let port_accessible = IdbRecovery::is_companion_port_accessible().await;
                            
                            eprintln!("Calibration: Timeout recovery - Companion running: {}, Port accessible: {}", 
                                companion_running, port_accessible);
                            
                            // Timeouts usually indicate stuck companion
                            if companion_running {
                                eprintln!("Calibration: Using stuck companion recovery for timeout...");
                                if let Ok(_) = self.idb_recovery.recover_stuck_companion().await {
                                    eprintln!("Calibration: Stuck companion recovery completed after timeout");
                                }
                            } else {
                                eprintln!("Calibration: Using general recovery for timeout...");
                                if let Ok(_) = self.idb_recovery.attempt_recovery().await {
                                    eprintln!("Calibration: General recovery completed after timeout");
                                }
                            }
                            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                        }
                        
                        self.update_idb_status(session_id, IdbStatus {
                            connected: false,
                            last_health_check: Some(chrono::Utc::now()),
                            last_error: Some("Tap operation timed out".to_string()),
                            companion_running: false,
                        }).await;
                    }
                }
                
                // Give more time between taps for the UI to update
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
            
            // Wait for verification results
            eprintln!("Calibration: Waiting for verification results...");
            eprintln!("Calibration: Looking for results at:");
            eprintln!("  - App Documents: via simctl get_app_container");
            eprintln!("  - Temp file: /tmp/calibration_{}.json", device_id);
            
            match verification_reader.wait_for_results(10) {
                Ok(results) => {
                    eprintln!("Calibration: Got verification results!");
                    eprintln!("  Accuracy: {:.1}%", results.accuracy_percentage());
                    
                    if let Some(avg_offset) = results.average_offset() {
                        eprintln!("  Average offset: ({:.1}, {:.1})", avg_offset.x, avg_offset.y);
                        
                        // If offset is small enough, we're done
                        if avg_offset.distance_to(&Coordinate { x: 0.0, y: 0.0 }) < 5.0 {
                            eprintln!("Calibration: Offset within tolerance, calibration complete!");
                            coordinate_offset = avg_offset;
                            break;
                        }
                        
                        // Otherwise, adjust and retry
                        coordinate_offset.x += avg_offset.x;
                        coordinate_offset.y += avg_offset.y;
                        eprintln!("Calibration: Adjusting offset for next attempt");
                    } else {
                        eprintln!("Calibration: No valid taps recorded, continuing with current offset");
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("Calibration: No verification data available: {}", e);
                    eprintln!("Calibration: Proceeding without verification");
                    break;
                }
            }
        }
        
        
        // Get the tap count from the session
        let tap_count = {
            let sessions = self.active_calibrations.read().await;
            sessions.get(session_id)
                .map(|s| s.tap_count)
                .unwrap_or(0)
        };
        
        // Phase 5: Generate calibration result with offset
        const EXPECTED_TAPS: u32 = 5; // We expect 5 taps for calibration
        let calibration_result = self.generate_calibration_result_with_offset(
            device_params.clone(),
            coordinate_offset,
            tap_count,
            EXPECTED_TAPS,
        );
        
        // Phase 6: Store results
        let config = CalibrationConfig {
            device_id: device_id.clone(),
            device_type: device_params.device_name.clone(),
            screen_size: device_params.screen_resolution.clone(),
            safe_area: SafeArea { top: 0.0, bottom: 0.0, left: 0.0, right: 0.0 },
            scale_factor: device_params.pixel_density,
            calibration_version: "1.0".to_string(),
            last_calibrated: chrono::Utc::now(),
        };
        
        // Only store calibration if it was successful
        if calibration_result.success {
            eprintln!("Calibration: Success! Storing calibration result with {} successful taps", tap_count);
            self.data_store.store_calibration(&device_id, config, calibration_result)?;
            self.update_session_status(session_id, CalibrationStatus::Complete).await;
            Ok(())
        } else {
            let error_msg = format!(
                "Calibration failed: Only {} of {} taps succeeded ({}% accuracy)",
                tap_count, EXPECTED_TAPS, calibration_result.validation_report.accuracy_percentage
            );
            eprintln!("Calibration: {}", error_msg);
            self.update_session_status(session_id, CalibrationStatus::Failed(error_msg.clone())).await;
            Err(CalibrationError::ValidationError(error_msg))
        }
    }
    
    fn generate_calibration_result_with_offset(
        &self,
        device_profile: DeviceProfile,
        coordinate_offset: Coordinate,
        tap_count: u32,
        expected_taps: u32,
    ) -> CalibrationResult {
        let mut interaction_adjustments = HashMap::new();
        let edge_cases = Vec::new();
        
        // Store the global coordinate offset
        interaction_adjustments.insert("global".to_string(), InteractionAdjustment {
            element_type: "global".to_string(),
            tap_offset: Some((coordinate_offset.x, coordinate_offset.y)),
            requires_double_tap: false,
            requires_long_press: false,
            custom_delay_ms: None,
        });
        
        // Add device-specific adjustments
        interaction_adjustments.insert("button".to_string(), InteractionAdjustment {
            element_type: "button".to_string(),
            tap_offset: Some((coordinate_offset.x, coordinate_offset.y)),
            requires_double_tap: false,
            requires_long_press: false,
            custom_delay_ms: Some(50),
        });
        
        interaction_adjustments.insert("checkbox".to_string(), InteractionAdjustment {
            element_type: "checkbox".to_string(),
            tap_offset: Some((coordinate_offset.x + 2.0, coordinate_offset.y + 2.0)), // Additional offset for checkboxes
            requires_double_tap: false,
            requires_long_press: false,
            custom_delay_ms: Some(100),
        });
        
        // Create a validation report based on actual tap results
        let failed_taps = expected_taps.saturating_sub(tap_count);
        let accuracy = if expected_taps > 0 {
            (tap_count as f64 / expected_taps as f64) * 100.0
        } else {
            0.0
        };
        
        let mut issues = vec![];
        if tap_count == 0 {
            issues.push(ValidationIssue {
                element_id: "calibration_tap".to_string(),
                expected_result: format!("{} successful taps", expected_taps),
                actual_result: "No taps were successfully executed".to_string(),
                severity: IssueSeverity::Critical,
            });
        } else if tap_count < expected_taps {
            issues.push(ValidationIssue {
                element_id: "calibration_tap".to_string(),
                expected_result: format!("{} successful taps", expected_taps),
                actual_result: format!("Only {} taps succeeded", tap_count),
                severity: IssueSeverity::Major,
            });
        }
        
        let validation_report = ValidationReport {
            total_interactions: expected_taps as usize,
            successful_interactions: tap_count as usize,
            failed_interactions: failed_taps as usize,
            accuracy_percentage: accuracy,
            issues,
        };
        
        // Consider calibration successful if we achieved at least 60% of expected taps
        // AND we have at least one successful tap
        let success = accuracy >= 60.0 && tap_count > 0;
        
        eprintln!("Calibration: Creating result - tap_count: {}, expected: {}, accuracy: {:.1}%, success: {}", 
            tap_count, expected_taps, accuracy, success);
        
        CalibrationResult {
            success,
            device_profile,
            interaction_adjustments,
            edge_cases,
            validation_report,
        }
    }
    
    pub async fn get_calibration_status(&self, session_id: &str) -> Option<CalibrationStatusReport> {
        let sessions = self.active_calibrations.read().await;
        sessions.get(session_id).map(|session| {
            CalibrationStatusReport {
                session_id: session_id.to_string(),
                device_id: session.device_id.clone(),
                start_time: session.start_time,
                elapsed_seconds: (chrono::Utc::now() - session.start_time).num_seconds() as u64,
                status: match &session.status {
                    CalibrationStatus::Initializing => "initializing".to_string(),
                    CalibrationStatus::Validating => "validating".to_string(),
                    CalibrationStatus::Complete => "complete".to_string(),
                    CalibrationStatus::Failed(err) => format!("failed: {}", err),
                },
                idb_status: session.idb_status.clone(),
                last_tap_time: session.last_tap_time,
                tap_count: session.tap_count,
            }
        })
    }
    
    pub async fn enable_auto_monitoring(&self, enabled: bool) {
        let mut monitor = self.auto_monitor.write().await;
        monitor.enabled = enabled;
        
        if enabled {
            let server = self.clone();
            tokio::spawn(async move {
                server.run_auto_monitor().await;
            });
        }
    }
    
    async fn run_auto_monitor(&self) {
        loop {
            {
                let monitor = self.auto_monitor.read().await;
                if !monitor.enabled {
                    break;
                }
                
                // Check all calibrated devices
                let devices = self.data_store.list_calibrated_devices();
                for device in devices {
                    if !self.data_store.is_calibration_valid(
                        &device.device_id, 
                        monitor.recalibration_threshold_hours
                    ) {
                        // Trigger recalibration
                        if let Err(e) = self.start_calibration(device.device_id.clone(), None).await {
                            eprintln!("Failed to start auto-recalibration: {}", e);
                        }
                    }
                }
                
                // Wait for next check
                tokio::time::sleep(
                    tokio::time::Duration::from_secs(monitor.check_interval_hours * 3600)
                ).await;
            }
        }
    }
    
    async fn update_session_status(&self, session_id: &str, status: CalibrationStatus) {
        let mut sessions = self.active_calibrations.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            session.status = status;
        }
    }
    
    async fn mark_session_failed(&self, session_id: &str, error: String) {
        self.update_session_status(session_id, CalibrationStatus::Failed(error)).await;
    }
    
    pub async fn handle_request(&self, request: CalibrationRequest) -> CalibrationResponse {
        let api = CalibrationAPI::new(self.clone());
        api.handle_request(request).await
    }
}

impl Clone for CalibrationServer {
    fn clone(&self) -> Self {
        Self {
            data_store: Arc::clone(&self.data_store),
            active_calibrations: Arc::clone(&self.active_calibrations),
            auto_monitor: Arc::clone(&self.auto_monitor),
            #[cfg(target_os = "macos")]
            idb_recovery: Arc::clone(&self.idb_recovery),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationStatusReport {
    pub session_id: String,
    pub device_id: String,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub elapsed_seconds: u64,
    pub status: String,
    pub idb_status: IdbStatus,
    pub last_tap_time: Option<chrono::DateTime<chrono::Utc>>,
    pub tap_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdbStatus {
    pub connected: bool,
    pub last_health_check: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
    pub companion_running: bool,
}

pub struct CalibrationAPI {
    server: CalibrationServer,
}

impl CalibrationAPI {
    pub fn new(server: CalibrationServer) -> Self {
        Self { server }
    }
    
    pub async fn handle_request(&self, request: CalibrationRequest) -> CalibrationResponse {
        match request {
            CalibrationRequest::StartCalibration { device_id, reference_bundle_id } => {
                match self.server.start_calibration(device_id, reference_bundle_id).await {
                    Ok(session_id) => CalibrationResponse::SessionStarted { session_id },
                    Err(e) => CalibrationResponse::Error { message: e.to_string() },
                }
            }
            CalibrationRequest::GetStatus { session_id } => {
                match self.server.get_calibration_status(&session_id).await {
                    Some(status) => CalibrationResponse::Status(status),
                    None => CalibrationResponse::Error { 
                        message: "Session not found".to_string() 
                    },
                }
            }
            CalibrationRequest::GetCalibration { device_id } => {
                match self.server.data_store.export_calibration(&device_id) {
                    Ok(data) => CalibrationResponse::CalibrationData { data },
                    Err(e) => CalibrationResponse::Error { message: e.to_string() },
                }
            }
            CalibrationRequest::EnableAutoMonitoring { enabled } => {
                self.server.enable_auto_monitoring(enabled).await;
                CalibrationResponse::Success
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CalibrationRequest {
    StartCalibration {
        device_id: String,
        reference_bundle_id: Option<String>,
    },
    GetStatus {
        session_id: String,
    },
    GetCalibration {
        device_id: String,
    },
    EnableAutoMonitoring {
        enabled: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CalibrationResponse {
    SessionStarted { session_id: String },
    Status(CalibrationStatusReport),
    CalibrationData { data: String },
    Success,
    Error { message: String },
}