use std::path::Path;

fn main() {
    // Verify that all required model files exist at compile time
    let model_files = vec![
        "models/model.onnx",
        "models/tokenizer.json",
        "models/config.json",
        "models/special_tokens_map.json",
        "models/tokenizer_config.json",
    ];

    for file in model_files {
        let path = Path::new(file);
        if !path.exists() {
            panic!(
                "Required model file '{}' not found. Please ensure all model files are present in the models/ directory.",
                file
            );
        }
    }

    // Tell Cargo to rerun this build script if any model file changes
    println!("cargo:rerun-if-changed=models/");
}

