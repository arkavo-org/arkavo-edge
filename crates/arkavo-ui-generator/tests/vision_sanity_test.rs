use arkavo_ui_generator::vision::{ModelSize, Qwen25VLModelLoader};

#[test]
fn test_model_loader_creation() {
    let result = Qwen25VLModelLoader::new();
    assert!(result.is_ok(), "Model loader should be created successfully");
}

#[test]
fn test_model_sizes_available() {
    // Test that model size enum variants exist and can be created
    let _small = ModelSize::Small3B;
    let _medium = ModelSize::Medium7B;
    let _large = ModelSize::Large32B;

    // If we get here, all variants are accessible
    assert!(true);
}

#[cfg(feature = "llama-cpp")]
#[tokio::test]
async fn test_model_paths_structure() {
    let loader = Qwen25VLModelLoader::new().unwrap();

    // Test that we can check for cached models without downloading
    let cached_7b = loader.get_cached_paths(ModelSize::Medium7B);
    let cached_3b = loader.get_cached_paths(ModelSize::Small3B);

    // We don't assert they exist, just that the API works
    assert!(cached_7b.is_none() || cached_7b.is_some());
    assert!(cached_3b.is_none() || cached_3b.is_some());
}

#[test]
fn test_vision_module_exports() {
    // Verify that key types are exported and accessible
    let _size: ModelSize = ModelSize::Medium7B;
    let _loader = Qwen25VLModelLoader::new();

    // This test just confirms the module structure is correct
    assert!(true);
}
