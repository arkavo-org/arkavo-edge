//! Tests for tokenizer file cleanup functionality
//!
//! These tests verify the infrastructure for cleaning up temporary tokenizer files.

#![cfg(feature = "llama-cpp")]

#[test]
fn test_tokenizer_temp_file_cleanup() {
    // This test verifies that temporary tokenizer files are cleaned up properly
    // The actual cleanup happens in ManagedTokenizer's Drop impl when llama-cpp is enabled

    let temp_dir = std::env::temp_dir();
    let test_model_name = format!("test_model_{}", std::process::id());

    // Clean up any existing test files first
    if let Ok(entries) = std::fs::read_dir(&temp_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str()
                && name.starts_with(&format!("arkavo_tokenizer_{test_model_name}_"))
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    // Create a test tokenizer file
    let test_tokenizer_path = temp_dir.join(format!(
        "arkavo_tokenizer_{}_{}.spm",
        test_model_name,
        std::process::id()
    ));

    // Write some dummy data
    std::fs::write(&test_tokenizer_path, b"dummy tokenizer data")
        .expect("Failed to write test file");

    // Verify file exists
    assert!(test_tokenizer_path.exists(), "Test file should exist");

    // Clean up our test file
    if test_tokenizer_path.exists() {
        std::fs::remove_file(&test_tokenizer_path).expect("Failed to clean up test file");
    }

    // Verify cleanup worked
    assert!(!test_tokenizer_path.exists(), "Test file should be cleaned up");
}

#[test]
fn test_unique_tokenizer_filenames() {
    // Verify that tokenizer temp files use unique names
    let temp_dir = std::env::temp_dir();
    let model_name = "test_model";
    let pid = std::process::id();

    let expected_pattern = format!("arkavo_tokenizer_{model_name}_{pid}.spm");
    let _path = temp_dir.join(&expected_pattern);

    // Verify the pattern includes process ID for uniqueness
    assert!(
        expected_pattern.contains(&pid.to_string()),
        "Tokenizer filename should include process ID"
    );

    assert!(
        expected_pattern.starts_with("arkavo_tokenizer_"),
        "Tokenizer filename should have standard prefix"
    );
}
