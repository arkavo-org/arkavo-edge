//! Lightweight file-based persistence for auto-learning state
//!
//! Persists `AutoLearnStats` and maintains an append-only audit log
//! of patch records in JSON Lines format.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AutoLearnError, AutoLearnResult};
use crate::learner::AutoLearnStats;

/// Status of a patch in the audit log
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PatchRecordStatus {
    /// Patch was broadcast to the network
    Broadcast,
    /// Patch was received from the network
    Received,
    /// Patch was accepted after verification
    Accepted,
    /// Patch was rejected after verification
    Rejected,
    /// Patch was synthesized in dry-run mode (not broadcast)
    DryRun,
}

/// A record of a patch event for audit logging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchRecord {
    /// The patch identifier
    pub patch_id: Uuid,
    /// When this event occurred
    pub timestamp: DateTime<Utc>,
    /// What happened to the patch
    pub status: PatchRecordStatus,
    /// Description of the signal that triggered synthesis
    pub signal_description: String,
    /// Summary of verification results
    pub verification_summary: String,
}

/// File-based persistence store for auto-learning state
pub struct AutoLearnStore {
    dir: PathBuf,
}

impl AutoLearnStore {
    /// Create a new store, creating the directory if needed
    pub fn new(dir: &Path) -> AutoLearnResult<Self> {
        fs::create_dir_all(dir)
            .map_err(|e| AutoLearnError::Internal(format!("Failed to create store dir: {e}")))?;
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    /// Save stats to `autolearn_stats.json`
    pub fn save_stats(&self, stats: &AutoLearnStats) -> AutoLearnResult<()> {
        let path = self.dir.join("autolearn_stats.json");
        let json = serde_json::to_string_pretty(stats)?;
        fs::write(path, json)
            .map_err(|e| AutoLearnError::Internal(format!("Failed to write stats: {e}")))?;
        Ok(())
    }

    /// Load stats from `autolearn_stats.json`, returns `Ok(None)` if file missing
    pub fn load_stats(&self) -> AutoLearnResult<Option<AutoLearnStats>> {
        let path = self.dir.join("autolearn_stats.json");
        if !path.exists() {
            return Ok(None);
        }
        let data = fs::read_to_string(path)
            .map_err(|e| AutoLearnError::Internal(format!("Failed to read stats: {e}")))?;
        let stats: AutoLearnStats = serde_json::from_str(&data)?;
        Ok(Some(stats))
    }

    /// Append a patch record to `patches.jsonl` (JSON Lines, append-only)
    pub fn append_patch_record(&self, record: &PatchRecord) -> AutoLearnResult<()> {
        let path = self.dir.join("patches.jsonl");
        let mut line = serde_json::to_string(record)?;
        line.push('\n');
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| AutoLearnError::Internal(format!("Failed to open patch log: {e}")))?;
        file.write_all(line.as_bytes())
            .map_err(|e| AutoLearnError::Internal(format!("Failed to write patch record: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir() -> PathBuf {
        std::env::temp_dir().join(format!("autolearn_test_{}", Uuid::new_v4()))
    }

    #[test]
    fn test_store_roundtrip() {
        let dir = test_dir();
        let store = AutoLearnStore::new(&dir).unwrap();

        let stats = AutoLearnStats {
            signals_processed: 10,
            syntheses_succeeded: 7,
            syntheses_failed: 3,
            patches_broadcast: 5,
            patches_dry_run: 2,
            patches_received: 4,
            patches_accepted: 3,
            patches_rejected: 1,
        };

        store.save_stats(&stats).unwrap();
        let loaded = store.load_stats().unwrap().unwrap();

        assert_eq!(loaded.signals_processed, 10);
        assert_eq!(loaded.syntheses_succeeded, 7);
        assert_eq!(loaded.syntheses_failed, 3);
        assert_eq!(loaded.patches_broadcast, 5);
        assert_eq!(loaded.patches_dry_run, 2);
        assert_eq!(loaded.patches_received, 4);
        assert_eq!(loaded.patches_accepted, 3);
        assert_eq!(loaded.patches_rejected, 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_stats_missing_file() {
        let dir = test_dir();
        let store = AutoLearnStore::new(&dir).unwrap();

        let result = store.load_stats().unwrap();
        assert!(result.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_patch_record_append() {
        let dir = test_dir();
        let store = AutoLearnStore::new(&dir).unwrap();

        let record1 = PatchRecord {
            patch_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            status: PatchRecordStatus::Broadcast,
            signal_description: "anomaly detected".to_string(),
            verification_summary: "passed=true".to_string(),
        };
        let record2 = PatchRecord {
            patch_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            status: PatchRecordStatus::Accepted,
            signal_description: "remote patch".to_string(),
            verification_summary: "passed=true".to_string(),
        };

        store.append_patch_record(&record1).unwrap();
        store.append_patch_record(&record2).unwrap();

        let content = fs::read_to_string(dir.join("patches.jsonl")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        let parsed1: PatchRecord = serde_json::from_str(lines[0]).unwrap();
        let parsed2: PatchRecord = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(parsed1.patch_id, record1.patch_id);
        assert_eq!(parsed2.patch_id, record2.patch_id);

        let _ = fs::remove_dir_all(&dir);
    }
}
