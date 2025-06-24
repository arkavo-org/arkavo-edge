use std::fs;
use std::path::Path;
use std::process::Command;

const MODEL_FILES: &[(&str, &str)] = &[
    (
        "model.onnx",
        "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx/resolve/main/model.onnx",
    ),
    (
        "tokenizer.json",
        "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx/resolve/main/tokenizer.json",
    ),
    (
        "config.json",
        "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx/resolve/main/config.json",
    ),
    (
        "special_tokens_map.json",
        "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx/resolve/main/special_tokens_map.json",
    ),
    (
        "tokenizer_config.json",
        "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx/resolve/main/tokenizer_config.json",
    ),
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

            // Try to download using curl (more compatible with various environments)
            let output = Command::new("curl")
                .args(&[
                    "-L", // Follow redirects
                    "-f", // Fail on HTTP errors
                    "-s", // Silent mode
                    "-o",
                    file_path.to_str().unwrap(),
                    url,
                ])
                .output();

            match output {
                Ok(result) => {
                    if !result.status.success() {
                        // If curl fails, try wget as fallback
                        let wget_output = Command::new("wget")
                            .args(&[
                                "-q", // Quiet mode
                                "-O",
                                file_path.to_str().unwrap(),
                                url,
                            ])
                            .output();

                        if wget_output.is_err() || !wget_output.unwrap().status.success() {
                            panic!(
                                "Failed to download {} - please ensure curl or wget is available",
                                filename
                            );
                        }
                    }
                    println!("cargo:warning=Downloaded: {}", filename);
                }
                Err(_) => {
                    // curl not available, try wget
                    let wget_output = Command::new("wget")
                        .args(&[
                            "-q", // Quiet mode
                            "-O",
                            file_path.to_str().unwrap(),
                            url,
                        ])
                        .output();

                    match wget_output {
                        Ok(result) if result.status.success() => {
                            println!("cargo:warning=Downloaded: {}", filename);
                        }
                        _ => {
                            panic!(
                                "Failed to download {} - please ensure curl or wget is available",
                                filename
                            );
                        }
                    }
                }
            }
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