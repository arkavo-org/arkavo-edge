use super::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct CalibrationDataStore {
    storage_path: PathBuf,
    cache: Arc<Mutex<HashMap<String, CalibrationConfig>>>,
}

impl CalibrationDataStore {
    pub fn new(storage_path: impl AsRef<Path>) -> Result<Self, CalibrationError> {
        let storage_path = storage_path.as_ref().to_path_buf();
        
        // Create directory if it doesn't exist
        fs::create_dir_all(&storage_path)?;
        
        let mut store = Self {
            storage_path,
            cache: Arc::new(Mutex::new(HashMap::new())),
        };
        
        // Load existing calibrations
        store.load_all_calibrations()?;
        
        Ok(store)
    }
    
    pub fn store_calibration(
        &self,
        device_id: &str,
        config: CalibrationConfig,
        result: CalibrationResult,
    ) -> Result<(), CalibrationError> {
        eprintln!("[CalibrationDataStore] Storing calibration for device {}", device_id);
        eprintln!("[CalibrationDataStore] Result - success: {}, successful_interactions: {}/{}", 
            result.success, 
            result.validation_report.successful_interactions,
            result.validation_report.total_interactions
        );
        
        // Store config
        let config_path = self.get_config_path(device_id);
        let config_data = serde_json::to_string_pretty(&config)?;
        fs::write(&config_path, config_data)?;
        
        // Store result
        let result_path = self.get_result_path(device_id);
        let result_data = serde_json::to_string_pretty(&result)?;
        fs::write(&result_path, result_data)?;
        
        eprintln!("[CalibrationDataStore] Wrote calibration result to: {}", result_path.display());
        
        // Update cache
        let mut cache = self.cache.lock().unwrap();
        let version = config.calibration_version.clone();
        cache.insert(device_id.to_string(), config);
        
        // Store versioned backup
        let timestamp = chrono::Utc::now().timestamp();
        let backup_path = self.storage_path
            .join("backups")
            .join(format!("{}_{}_{}.json", device_id, version, timestamp));
        fs::create_dir_all(backup_path.parent().unwrap())?;
        
        let full_data = CalibrationData {
            config: cache.get(device_id).cloned().unwrap(),
            result,
        };
        
        let backup_data = serde_json::to_string_pretty(&full_data)?;
        fs::write(&backup_path, backup_data)?;
        
        Ok(())
    }
    
    pub fn get_calibration(&self, device_id: &str) -> Option<CalibrationConfig> {
        let cache = self.cache.lock().unwrap();
        cache.get(device_id).cloned()
    }
    
    pub fn get_latest_result(&self, device_id: &str) -> Result<CalibrationResult, CalibrationError> {
        let result_path = self.get_result_path(device_id);
        eprintln!("[CalibrationDataStore] Reading calibration result from: {}", result_path.display());
        let data = fs::read_to_string(&result_path)?;
        let result: CalibrationResult = serde_json::from_str(&data)?;
        eprintln!("[CalibrationDataStore] Retrieved result - success: {}, successful_interactions: {}/{}", 
            result.success,
            result.validation_report.successful_interactions,
            result.validation_report.total_interactions
        );
        Ok(result)
    }
    
    pub fn is_calibration_valid(&self, device_id: &str, max_age_hours: u64) -> bool {
        if let Some(config) = self.get_calibration(device_id) {
            let age = chrono::Utc::now() - config.last_calibrated;
            age.num_hours() < max_age_hours as i64
        } else {
            false
        }
    }
    
    pub fn get_adjustment_for_element(
        &self,
        device_id: &str,
        element_type: &str,
    ) -> Option<InteractionAdjustment> {
        let result = self.get_latest_result(device_id).ok()?;
        result.interaction_adjustments.get(element_type).cloned()
    }
    
    pub fn list_calibrated_devices(&self) -> Vec<DeviceSummary> {
        // First, collect all the device data we need while holding the lock
        let device_data: Vec<(String, String, chrono::DateTime<chrono::Utc>)> = {
            let cache = self.cache.lock().unwrap();
            cache.iter().map(|(id, config)| {
                (id.clone(), config.device_type.clone(), config.last_calibrated)
            }).collect()
        };
        
        // Now check validity without holding the lock
        device_data.into_iter().map(|(id, device_type, last_calibrated)| {
            DeviceSummary {
                device_id: id.clone(),
                device_type,
                last_calibrated,
                is_valid: self.is_calibration_valid(&id, 24 * 7), // 1 week
            }
        }).collect()
    }
    
    pub fn export_calibration(&self, device_id: &str) -> Result<String, CalibrationError> {
        let config = self.get_calibration(device_id)
            .ok_or_else(|| CalibrationError::DeviceNotFound(device_id.to_string()))?;
        let result = self.get_latest_result(device_id)?;
        
        let export_data = CalibrationExport {
            export_version: "1.0".to_string(),
            exported_at: chrono::Utc::now(),
            device_config: config,
            calibration_result: result,
        };
        
        Ok(serde_json::to_string_pretty(&export_data)?)
    }
    
    pub fn import_calibration(&self, data: &str) -> Result<String, CalibrationError> {
        let import_data: CalibrationExport = serde_json::from_str(data)?;
        let device_id = import_data.device_config.device_id.clone();
        
        self.store_calibration(
            &device_id,
            import_data.device_config,
            import_data.calibration_result,
        )?;
        
        Ok(device_id)
    }
    
    pub fn clear_old_backups(&self, days_to_keep: u64) -> Result<usize, CalibrationError> {
        let backup_dir = self.storage_path.join("backups");
        if !backup_dir.exists() {
            return Ok(0);
        }
        
        let cutoff = chrono::Utc::now() - chrono::Duration::days(days_to_keep as i64);
        let mut removed = 0;
        
        for entry in fs::read_dir(&backup_dir)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            
            if let Ok(modified) = metadata.modified() {
                let modified_time = chrono::DateTime::<chrono::Utc>::from(modified);
                if modified_time < cutoff {
                    fs::remove_file(entry.path())?;
                    removed += 1;
                }
            }
        }
        
        Ok(removed)
    }
    
    fn load_all_calibrations(&mut self) -> Result<(), CalibrationError> {
        let mut cache = self.cache.lock().unwrap();
        
        if let Ok(entries) = fs::read_dir(&self.storage_path) {
            for entry in entries {
                let entry = entry?;
                let path = entry.path();
                
                if path.extension().and_then(|s| s.to_str()) == Some("json") &&
                   path.file_stem().and_then(|s| s.to_str()).map(|s| s.ends_with("_config")).unwrap_or(false) {
                    if let Ok(data) = fs::read_to_string(&path) {
                        if let Ok(config) = serde_json::from_str::<CalibrationConfig>(&data) {
                            cache.insert(config.device_id.clone(), config);
                        }
                    }
                }
            }
        }
        
        Ok(())
    }
    
    fn get_config_path(&self, device_id: &str) -> PathBuf {
        self.storage_path.join(format!("{}_config.json", device_id))
    }
    
    fn get_result_path(&self, device_id: &str) -> PathBuf {
        self.storage_path.join(format!("{}_result.json", device_id))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationData {
    pub config: CalibrationConfig,
    pub result: CalibrationResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationExport {
    pub export_version: String,
    pub exported_at: chrono::DateTime<chrono::Utc>,
    pub device_config: CalibrationConfig,
    pub calibration_result: CalibrationResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceSummary {
    pub device_id: String,
    pub device_type: String,
    pub last_calibrated: chrono::DateTime<chrono::Utc>,
    pub is_valid: bool,
}

pub struct CalibrationCache {
    memory_cache: Arc<Mutex<HashMap<String, CachedCalibration>>>,
    max_cache_size: usize,
}

#[derive(Clone)]
struct CachedCalibration {
    config: CalibrationConfig,
    result: CalibrationResult,
    accessed_at: chrono::DateTime<chrono::Utc>,
}

impl CalibrationCache {
    pub fn new(max_cache_size: usize) -> Self {
        Self {
            memory_cache: Arc::new(Mutex::new(HashMap::new())),
            max_cache_size,
        }
    }
    
    pub fn get(&self, device_id: &str) -> Option<(CalibrationConfig, CalibrationResult)> {
        let mut cache = self.memory_cache.lock().unwrap();
        
        if let Some(cached) = cache.get_mut(device_id) {
            cached.accessed_at = chrono::Utc::now();
            Some((cached.config.clone(), cached.result.clone()))
        } else {
            None
        }
    }
    
    pub fn put(&self, device_id: String, config: CalibrationConfig, result: CalibrationResult) {
        let mut cache = self.memory_cache.lock().unwrap();
        
        // Evict least recently used if at capacity
        if cache.len() >= self.max_cache_size {
            if let Some((lru_id, _)) = cache.iter()
                .min_by_key(|(_, cached)| cached.accessed_at)
                .map(|(id, cached)| (id.clone(), cached.accessed_at)) {
                cache.remove(&lru_id);
            }
        }
        
        cache.insert(device_id, CachedCalibration {
            config,
            result,
            accessed_at: chrono::Utc::now(),
        });
    }
    
    pub fn invalidate(&self, device_id: &str) {
        let mut cache = self.memory_cache.lock().unwrap();
        cache.remove(device_id);
    }
    
    pub fn clear(&self) {
        let mut cache = self.memory_cache.lock().unwrap();
        cache.clear();
    }
}