use super::*;
use std::fs;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TapVerification {
    #[serde(default = "chrono::Utc::now")]
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub expected: Coordinate,
    pub actual: Coordinate,
    #[serde(alias = "target_hit", alias = "targetHit", default = "default_true")]
    pub target_hit: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coordinate {
    pub x: f64,
    pub y: f64,
}

impl Coordinate {
    pub fn offset_from(&self, other: &Coordinate) -> Coordinate {
        Coordinate {
            x: self.x - other.x,
            y: self.y - other.y,
        }
    }
    
    pub fn distance_to(&self, other: &Coordinate) -> f64 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationResults {
    #[serde(default)]
    pub device_info: String,
    pub screen_size: ScreenSize,
    #[serde(alias = "tap_events", alias = "tapEvents")]
    pub tap_events: Vec<TapVerification>,
    #[serde(alias = "calibration_complete", alias = "calibrationComplete")]
    pub calibration_complete: bool,
}

impl CalibrationResults {
    pub fn average_offset(&self) -> Option<Coordinate> {
        let valid_taps: Vec<_> = self.tap_events
            .iter()
            .filter(|t| t.target_hit)
            .collect();
            
        if valid_taps.is_empty() {
            return None;
        }
        
        let sum_x: f64 = valid_taps.iter()
            .map(|t| t.actual.x - t.expected.x)
            .sum();
            
        let sum_y: f64 = valid_taps.iter()
            .map(|t| t.actual.y - t.expected.y)
            .sum();
            
        Some(Coordinate {
            x: sum_x / valid_taps.len() as f64,
            y: sum_y / valid_taps.len() as f64,
        })
    }
    
    pub fn accuracy_percentage(&self) -> f64 {
        if self.tap_events.is_empty() {
            return 0.0;
        }
        
        let hits = self.tap_events.iter().filter(|t| t.target_hit).count();
        (hits as f64 / self.tap_events.len() as f64) * 100.0
    }
}

pub struct VerificationReader {
    device_id: String,
}

impl VerificationReader {
    pub fn new(device_id: String) -> Self {
        Self { device_id }
    }
    
    pub fn read_calibration_results(&self) -> Result<CalibrationResults, CalibrationError> {
        // Try multiple locations where the app might write results
        let paths = vec![
            // Shared container (if app groups are set up)
            self.get_shared_container_path(),
            // App's documents directory via simctl
            self.get_app_documents_path(),
            // Fallback to temp directory
            PathBuf::from("/tmp").join(format!("calibration_{}.json", self.device_id)),
        ];
        
        eprintln!("[VerificationReader] Looking for calibration results...");
        for path in &paths {
            eprintln!("[VerificationReader] Checking path: {}", path.display());
            if path.exists() {
                eprintln!("[VerificationReader] Found results at: {}", path.display());
                let data = fs::read_to_string(&path)?;
                eprintln!("[VerificationReader] Raw data: {}", data);
                let results: CalibrationResults = serde_json::from_str(&data)?;
                
                eprintln!("[VerificationReader] Parsed results: {} tap events", results.tap_events.len());
                
                // Clean up after reading
                let _ = fs::remove_file(&path);
                
                return Ok(results);
            }
        }
        
        eprintln!("[VerificationReader] No calibration results found at any location");
        Err(CalibrationError::ValidationError(
            "No calibration results found. Ensure reference app is running in calibration mode.".to_string()
        ))
    }
    
    pub fn wait_for_results(&self, timeout_secs: u64) -> Result<CalibrationResults, CalibrationError> {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);
        
        loop {
            if start.elapsed() > timeout {
                return Err(CalibrationError::ValidationError(
                    format!("Timeout waiting for calibration results after {} seconds", timeout_secs)
                ));
            }
            
            match self.read_calibration_results() {
                Ok(results) => return Ok(results),
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
        }
    }
    
    fn get_shared_container_path(&self) -> PathBuf {
        // This would need to match the app group ID in the iOS app
        PathBuf::from(format!(
            "/Users/{}/Library/Developer/CoreSimulator/Devices/{}/data/Containers/Shared/AppGroup",
            std::env::var("USER").unwrap_or_default(),
            self.device_id
        )).join("group.arkavo.calibration/calibration_results.json")
    }
    
    fn get_app_documents_path(&self) -> PathBuf {
        // Use simctl to get app container path
        eprintln!("[VerificationReader] Getting app container path for device: {}", self.device_id);
        if let Ok(output) = std::process::Command::new("xcrun")
            .args(["simctl", "get_app_container", &self.device_id, "com.arkavo.reference", "data"])
            .output() 
        {
            if output.status.success() {
                let container_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                eprintln!("[VerificationReader] App container path: {}", container_path);
                return PathBuf::from(container_path)
                    .join("Documents/calibration_results.json");
            } else {
                eprintln!("[VerificationReader] Failed to get app container: {}", 
                    String::from_utf8_lossy(&output.stderr));
            }
        }
        
        PathBuf::from("/tmp/calibration_not_found.json")
    }
}

pub fn apply_calibration_offset(
    original: &Coordinate,
    offset: &Coordinate,
) -> Coordinate {
    Coordinate {
        x: original.x - offset.x, // Subtract offset to correct
        y: original.y - offset.y,
    }
}