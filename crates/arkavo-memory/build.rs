use std::fs;
use std::path::Path;

const MODEL_FILES: &[(&str, &str)] = &[
    ("model.onnx", "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx/resolve/main/model.onnx"),
    ("tokenizer.json", "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx/resolve/main/tokenizer.json"),
    ("config.json", "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx/resolve/main/config.json"),
    ("special_tokens_map.json", "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx/resolve/main/special_tokens_map.json"),
    ("tokenizer_config.json", "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx/resolve/main/tokenizer_config.json"),
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    
    // Create models directory if it doesn't exist
    let models_dir = Path::new("models");
    if !models_dir.exists() {
        fs::create_dir_all(models_dir).expect("Failed to create models directory");
    }
    
    // Download model files if they don't exist
    for (filename, url) in MODEL_FILES {
        let file_path = models_dir.join(filename);
        
        if !file_path.exists() {
            println!("cargo:warning=Downloading model file: {}", filename);
            
            // Download the file
            let response = ureq::get(url)
                .call()
                .unwrap_or_else(|e| panic!("Failed to download {}: {}", filename, e));
            
            // Save to file
            let mut file = fs::File::create(&file_path)
                .unwrap_or_else(|e| panic!("Failed to create {}: {}", filename, e));
            
            std::io::copy(&mut response.into_reader(), &mut file)
                .unwrap_or_else(|e| panic!("Failed to write {}: {}", filename, e));
            
            println!("cargo:warning=Downloaded: {}", filename);
        }
    }
    
    // Verify all files exist
    for (filename, _) in MODEL_FILES {
        let file_path = models_dir.join(filename);
        if !file_path.exists() {
            panic!(
                "Required model file '{}' not found after download attempt",
                file_path.display()
            );
        }
    }
}