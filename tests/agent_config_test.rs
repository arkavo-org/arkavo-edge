use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;

#[tokio::test]
async fn test_agent_config_backup_creation() {
    // Create a temporary directory for the test
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create initial AGENTS.md
    let initial_content = r#"# AGENTS.md

## test-agent
purpose: Test agent for configuration management
model: gpt-4
listen: 0.0.0.0:8765
"#;

    fs::write(temp_path.join("AGENTS.md"), initial_content).unwrap();

    // Update the configuration
    let updated_content = r#"# AGENTS.md

## test-agent
purpose: Updated test agent for configuration management
model: gpt-4-turbo
listen: 0.0.0.0:8765
features:
  - configuration-management
  - backup-recovery
"#;

    // Write to temporary file first (simulating atomic update)
    let temp_file = temp_path.join("AGENTS.md.tmp");
    fs::write(&temp_file, updated_content).unwrap();
    fs::rename(&temp_file, temp_path.join("AGENTS.md")).unwrap();

    // Verify the update was successful
    let final_content = fs::read_to_string(temp_path.join("AGENTS.md")).unwrap();
    assert_eq!(final_content, updated_content);

    // Check that we can create a backup directory
    let backup_dir = temp_path.join(".agents.md.backup");
    fs::create_dir_all(&backup_dir).unwrap();
    assert!(backup_dir.exists());

    // Create a backup file
    let backup_file = backup_dir.join("AGENTS.md.20250810_120000.backup");
    fs::write(&backup_file, initial_content).unwrap();
    assert!(backup_file.exists());

    // Test that we can restore from backup
    fs::copy(&backup_file, temp_path.join("AGENTS.md")).unwrap();
    let restored_content = fs::read_to_string(temp_path.join("AGENTS.md")).unwrap();
    assert_eq!(restored_content, initial_content);
}

#[tokio::test]
async fn test_agent_config_version_tracking() {
    use sha2::{Digest, Sha256};

    let content1 = "# AGENTS.md\n## agent1\npurpose: test\n";
    let content2 = "# AGENTS.md\n## agent1\npurpose: test updated\n";

    // Calculate SHA256 hashes
    let mut hasher1 = Sha256::new();
    hasher1.update(content1.as_bytes());
    let hash1 = format!("{:x}", hasher1.finalize());

    let mut hasher2 = Sha256::new();
    hasher2.update(content2.as_bytes());
    let hash2 = format!("{:x}", hasher2.finalize());

    // Verify different content produces different hashes
    assert_ne!(hash1, hash2);

    // Verify same content produces same hash
    let mut hasher3 = Sha256::new();
    hasher3.update(content1.as_bytes());
    let hash3 = format!("{:x}", hasher3.finalize());
    assert_eq!(hash1, hash3);
}

#[tokio::test]
async fn test_agent_config_backup_cleanup() {
    let temp_dir = TempDir::new().unwrap();
    let backup_dir = temp_dir.path().join(".agents.md.backup");
    fs::create_dir_all(&backup_dir).unwrap();

    // Create 15 backup files
    for i in 0..15 {
        let filename = format!("AGENTS.md.2025081012{:02}00.backup", i);
        let backup_file = backup_dir.join(&filename);
        fs::write(&backup_file, format!("backup content {}", i)).unwrap();
    }

    // Verify all 15 files exist
    let entries: Vec<_> = fs::read_dir(&backup_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 15);

    // Simulate cleanup logic (keep only last 10)
    let mut backups: Vec<_> = fs::read_dir(&backup_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|s| s.ends_with(".backup"))
                .unwrap_or(false)
        })
        .collect();

    // Sort by modification time
    backups.sort_by_key(|e| e.metadata().unwrap().modified().unwrap());

    // Remove oldest files if more than 10
    if backups.len() > 10 {
        for backup in backups.iter().take(backups.len() - 10) {
            fs::remove_file(backup.path()).unwrap();
        }
    }

    // Verify only 10 files remain
    let remaining: Vec<_> = fs::read_dir(&backup_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(remaining.len(), 10);
}

#[tokio::test]
async fn test_agent_config_atomic_update() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("AGENTS.md");
    let temp_path = temp_dir.path().join("AGENTS.md.tmp");

    // Write initial content
    let initial_content = "initial content";
    fs::write(&config_path, initial_content).unwrap();

    // Atomic update process
    let new_content = "updated content";
    fs::write(&temp_path, new_content).unwrap();
    
    // Verify temp file exists and original is unchanged
    assert!(temp_path.exists());
    assert_eq!(fs::read_to_string(&config_path).unwrap(), initial_content);

    // Perform atomic rename
    fs::rename(&temp_path, &config_path).unwrap();

    // Verify update succeeded and temp file is gone
    assert!(!temp_path.exists());
    assert_eq!(fs::read_to_string(&config_path).unwrap(), new_content);
}

#[tokio::test]
async fn test_agent_config_last_known_good() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("AGENTS.md");
    let last_known_good = temp_dir.path().join("AGENTS.md.last-known-good");

    // Write initial config
    let good_content = "# Working configuration\n";
    fs::write(&config_path, good_content).unwrap();

    // Save as last-known-good
    fs::copy(&config_path, &last_known_good).unwrap();

    // Simulate bad update
    let bad_content = "# Broken configuration";
    fs::write(&config_path, bad_content).unwrap();

    // Restore from last-known-good
    fs::copy(&last_known_good, &config_path).unwrap();

    // Verify restoration
    let restored = fs::read_to_string(&config_path).unwrap();
    assert_eq!(restored, good_content);
}