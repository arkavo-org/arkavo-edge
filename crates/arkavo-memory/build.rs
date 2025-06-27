use std::env;
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

    // Skip downloads in docs.rs builds
    if env::var("DOCS_RS").is_ok() {
        return;
    }

    // Add rpath settings for zero-config ONNX Runtime loading
    #[cfg(feature = "embeddings")]
    {
        let target = env::var("TARGET").unwrap_or_default();

        if target.contains("apple") {
            // macOS: Look for dylibs relative to the executable
            println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
            println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path");
        } else if target.contains("linux") && !target.contains("musl") {
            // Linux glibc: Use $ORIGIN for relative paths
            println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
            println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib");
        }
        // For musl targets, we rely on static linking via ORT_STRATEGY=system-static
    }

    // Skip downloads if embeddings feature is not enabled
    if env::var("CARGO_FEATURE_EMBEDDINGS").is_err() {
        println!("cargo:warning=Skipping model downloads - embeddings feature not enabled");
        return;
    }

    // Also skip if we're building tests without the feature
    if env::var("CARGO_CFG_TEST").is_ok() && env::var("CARGO_FEATURE_EMBEDDINGS").is_err() {
        println!("cargo:warning=Skipping model downloads - building tests without embeddings");
        return;
    }

    // Skip downloads if feature flag is set (for CI environments)
    if env::var("CARGO_FEATURE_SKIP_MODEL_DOWNLOAD").is_ok() {
        println!("cargo:warning=Skipping model downloads due to skip-model-download feature");
        return;
    }

    // Create models directory if it doesn't exist
    let models_dir = Path::new("models");
    if !models_dir.exists() {
        fs::create_dir_all(models_dir).expect("Failed to create models directory");
    }

    // Check if we're in CI and might need special handling
    let is_ci = env::var("CI").is_ok();
    if is_ci {
        println!("cargo:warning=Running in CI environment");
    }

    // Download model files if they don't exist
    for (filename, url) in MODEL_FILES {
        let file_path = models_dir.join(filename);

        if !file_path.exists() {
            println!("cargo:warning=Downloading model file: {filename}");

            // Try to download using curl (more compatible with various environments)
            let curl_result = Command::new("curl")
                .args([
                    "-L", // Follow redirects
                    "-f", // Fail on HTTP errors
                    "-s", // Silent mode
                    "-o",
                    file_path.to_str().unwrap(),
                    url,
                ])
                .output();

            match curl_result {
                Ok(result) => {
                    if result.status.success() {
                        println!("cargo:warning=Downloaded via curl: {filename}");
                    } else {
                        // Try wget as fallback
                        try_wget(&file_path, url, filename);
                    }
                }
                Err(e) => {
                    println!("cargo:warning=curl not available: {e}");
                    // Try wget as fallback
                    try_wget(&file_path, url, filename);
                }
            }

            // Verify the file was downloaded
            if !file_path.exists() {
                if is_ci {
                    panic!(
                        "Failed to download {filename} in CI environment. \
                         Please ensure curl or wget is available in the build image, \
                         or pre-download model files and cache them."
                    );
                } else {
                    panic!(
                        "Failed to download {filename}. Please ensure curl or wget is available, \
                         or manually download from: {url}"
                    );
                }
            }
        }
    }

    // Verify all files exist and have content
    for (filename, _) in MODEL_FILES {
        let file_path = models_dir.join(filename);
        if !file_path.exists() {
            let path = file_path.display();
            panic!("Required model file '{path}' not found");
        }

        // Check file is not empty
        let metadata = fs::metadata(&file_path).expect("Failed to read file metadata");
        if metadata.len() == 0 {
            let path = file_path.display();
            panic!("Model file '{path}' is empty");
        }
    }
}

fn try_wget(file_path: &Path, url: &str, filename: &str) {
    let wget_result = Command::new("wget")
        .args([
            "-q", // Quiet mode
            "-O",
            file_path.to_str().unwrap(),
            url,
        ])
        .output();

    match wget_result {
        Ok(result) if result.status.success() => {
            println!("cargo:warning=Downloaded via wget: {filename}");
        }
        Ok(result) => {
            let status = result.status;
            println!("cargo:warning=wget failed with status: {status}");
            if !result.stderr.is_empty() {
                let stderr = String::from_utf8_lossy(&result.stderr);
                println!("cargo:warning=wget stderr: {stderr}");
            }
        }
        Err(e) => {
            println!("cargo:warning=wget not available: {e}");
        }
    }
}
