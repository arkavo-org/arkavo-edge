use std::env;
use std::fs::{self, File};
use std::io::{Write, BufWriter};
use std::path::Path;
use std::time::Instant;
use phf_codegen::Map as PhfMap;

// Load all merges and vocabulary entries for correct operation
// Previous limits removed to ensure full tokenizer functionality

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Instrumentation to detect any potential infinite loops
    let start_time = Instant::now();
    
    // Only rebuild if these specific files change
    println!("cargo:rerun-if-changed=models/tokenizer.json");
    println!("cargo:rerun-if-changed=models/config.json");
    println!("cargo:rerun-if-changed=build.rs");
    
    // Also rebuild if environment variables change
    println!("cargo:rerun-if-env-changed=OUT_DIR");
    
    // Print a message for debugging
    println!("cargo:info=Starting tokenizer build script...");

    // Get output directory from Cargo
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let dest_path = Path::new(&out_dir).join("tokenizer_static.rs");
    
    // Read tokenizer.json
    let models_dir = Path::new("models");
    let tokenizer_path = models_dir.join("tokenizer.json");
    println!("cargo:info=Reading tokenizer.json from {:?}...", tokenizer_path);
    
    // Use BufWriter for better performance with large files
    let file = File::create(&dest_path)?;
    let mut writer = BufWriter::new(file);
    
    // Handle missing tokenizer file with appropriate fallback
    if !tokenizer_path.exists() {
        println!("cargo:info=No tokenizer file found at {:?}, using defaults", tokenizer_path);
        generate_default_tokenizer(&mut writer)?;
        return Ok(());
    }
    
    // Stream read and parse the JSON to handle large files
    println!("cargo:info=Parsing tokenizer.json...");
    let tokenizer_json = fs::read_to_string(&tokenizer_path)
        .map_err(|e| format!("Failed to read tokenizer.json: {}", e))?;
        
    let tokenizer: serde_json::Value = serde_json::from_str(&tokenizer_json)
        .map_err(|e| format!("Failed to parse tokenizer.json: {}", e))?;
    
    // Validate the tokenizer has required fields
    validate_tokenizer(&tokenizer)?;
    
    // Extract special tokens
    extract_special_tokens(&tokenizer, &mut writer)?;
    
    // Extract vocabulary
    extract_vocabulary(&tokenizer, &mut writer)?;
    
    // Extract and write merges
    extract_merges(&tokenizer, &mut writer)?;
    
    // Generate tokenizer_data.rs with embedded tokenizer content in the OUT_DIR
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let tokenizer_data_path = Path::new(&out_dir).join("tokenizer_data.rs");
    println!("cargo:info=Generating embedded tokenizer data at {:?}...", tokenizer_data_path);
    
    // Read the tokenizer file content and serialize it for embedding
    let tokenizer_content = fs::read(&tokenizer_path)?;
    let tokenizer_config_path = models_dir.join("config.json");
    let config_content = if tokenizer_config_path.exists() {
        Some(fs::read(&tokenizer_config_path)?)
    } else {
        None
    };
    
    // Create the tokenizer_data.rs file with embedded data
    generate_tokenizer_data(&tokenizer_data_path, &tokenizer_content, config_content.as_deref())?;
    
    // Enable the feature flag for the generated tokenizer
    println!("cargo:rustc-cfg=feature=\"use_generated_tokenizer\"");
    
    // Look for and embed GGUF model file
    let possible_model_files = [
        "Qwen3-0.6B-Q4_K_M.gguf",  // Q4_K_M quantization
        "Qwen3-0.6B.gguf",         // Default unquantized
        "qwen3-0.6b-gguf.bin",     // Alternative naming
    ];
    
    let mut found_model = false;
    for model_name in possible_model_files {
        let model_path = models_dir.join(model_name);
        if model_path.exists() {
            println!("cargo:info=Found GGUF model file: {:?}", model_path);
            
            // Read the model file content
            println!("cargo:info=Reading model file (this may take some time)...");
            let model_content = fs::read(&model_path)?;
            
            // Generate embedded_model.rs
            let model_data_path = Path::new(&out_dir).join("embedded_model.rs");
            println!("cargo:info=Generating embedded model data at {:?}...", model_data_path);
            generate_model_data(&model_data_path, &model_content)?;
            
            println!("cargo:info=Successfully embedded model: {} bytes", model_content.len());
            
            // Write a marker file to indicate that the model is embedded
            // This is used by the embedded_model.rs module to conditionally include the model data
            let marker_path = Path::new(&out_dir).join("has_embedded_model");
            let mut marker_file = File::create(&marker_path)?;
            writeln!(marker_file, "// This file indicates that an embedded model is available")?;
            
            found_model = true;
            break;
        }
    }
    
    if !found_model {
        println!("cargo:warning=No GGUF model found in models directory, skipping model embedding");
        
        // Create an empty placeholder file anyway, to avoid build errors
        let model_data_path = Path::new(&out_dir).join("embedded_model.rs");
        let mut file = File::create(model_data_path)?;
        writeln!(file, "// Empty placeholder for embedded model")?;
        writeln!(file, "pub const EMBEDDED_MODEL: &[u8] = &[];")?;
    }
    
    // Flush the writer to ensure all data is written to disk
    writer.flush()?;
    
    // Log completion time for debugging
    let elapsed = start_time.elapsed();
    println!("cargo:info=Tokenizer build script completed in {:.2?}", elapsed);

    Ok(())
}

/// Generates a Rust source file with embedded tokenizer data
fn generate_tokenizer_data(
    path: &Path, 
    tokenizer_content: &[u8],
    config_content: Option<&[u8]>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = File::create(path)?;
    writeln!(file, "// Generated tokenizer data - DO NOT EDIT")?;
    writeln!(file, "// This file is automatically generated by build.rs")?;
    writeln!(file)?;
    
    // Write the tokenizer JSON data
    writeln!(file, "/// The raw tokenizer JSON data")?;
    writeln!(file, "pub const TOKENIZER_JSON: &[u8] = &[")?;
    
    // Write data in chunks with proper formatting
    for chunk in tokenizer_content.chunks(16) {
        write!(file, "    ")?;
        for &byte in chunk {
            write!(file, "{}, ", byte)?;
        }
        writeln!(file)?;
    }
    writeln!(file, "];")?;
    writeln!(file)?;
    
    // Write the config JSON data if available
    if let Some(config) = config_content {
        writeln!(file, "/// The raw model config JSON data")?;
        writeln!(file, "pub const CONFIG_JSON: &[u8] = &[")?;
        
        // Write data in chunks with proper formatting
        for chunk in config.chunks(16) {
            write!(file, "    ")?;
            for &byte in chunk {
                write!(file, "{}, ", byte)?;
            }
            writeln!(file)?;
        }
        writeln!(file, "];")?;
    } else {
        writeln!(file, "/// Empty placeholder for config (none found)")?;
        writeln!(file, "pub const CONFIG_JSON: &[u8] = &[];")?;
    }
    
    println!("cargo:info=Successfully generated tokenizer_data.rs with {} bytes of tokenizer data", 
             tokenizer_content.len());
    
    Ok(())
}

/// Generates a Rust source file with embedded GGUF model data
fn generate_model_data(
    path: &Path,
    model_content: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = File::create(path)?;
    writeln!(file, "// Embedded GGUF model - DO NOT EDIT")?;
    writeln!(file, "// This file is automatically generated by build.rs")?;
    writeln!(file)?;
    
    writeln!(file, "/// The raw GGUF model data")?;
    writeln!(file, "pub const EMBEDDED_MODEL: &[u8] = &[")?;
    
    // Write data in chunks with proper formatting
    for chunk in model_content.chunks(16) {
        write!(file, "    ")?;
        for &byte in chunk {
            write!(file, "{}, ", byte)?;
        }
        writeln!(file)?;
    }
    writeln!(file, "];")?;
    
    Ok(())
}

/// Validates that the tokenizer JSON has the required fields
fn validate_tokenizer(tokenizer: &serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
    // Check for the model object
    if !tokenizer["model"].is_object() {
        return Err("Tokenizer JSON missing 'model' object".into());
    }
    
    // Check for either vocab or tokens in the model
    if !tokenizer["model"]["vocab"].is_object() && !tokenizer["model"]["tokens"].is_object() {
        return Err("Tokenizer JSON missing 'vocab' or 'tokens' object in model".into());
    }
    
    // Check for merges (warn but don't fail if missing)
    if !tokenizer["model"]["merges"].is_array() {
        println!("cargo:info=Tokenizer JSON missing 'merges' array in model, no BPE merges will be available");
    }
    
    Ok(())
}

/// Generates a default minimal tokenizer when no file is available
fn generate_default_tokenizer(writer: &mut BufWriter<File>) -> Result<(), Box<dyn std::error::Error>> {
    writeln!(writer, "// Default tokenizer values (no tokenizer.json found)")?;
    writeln!(writer, "pub static MERGES: &[(&str, &str, usize)] = &[];")?;
    writeln!(writer, "pub const BOS_TOKEN_ID: u32 = 1;")?;
    writeln!(writer, "pub const EOS_TOKEN_ID: u32 = 2;")?;
    writeln!(writer, "pub const PAD_TOKEN_ID: u32 = 0;")?;
    writeln!(writer, "pub const UNK_TOKEN_ID: u32 = 3;")?;
    writeln!(writer, "pub fn get_vocab() -> &'static phf::Map<&'static str, u32> {{ &VOCAB }}")?;
    writeln!(writer, "pub fn get_id_to_token() -> &'static phf::Map<u32, &'static str> {{ &ID_TO_TOKEN }}")?;
    writeln!(writer, "static VOCAB: phf::Map<&'static str, u32> = phf::phf_map!{{")?;
    writeln!(writer, "    \"<pad>\" => 0,")?;
    writeln!(writer, "    \"<s>\" => 1,")?;
    writeln!(writer, "    \"</s>\" => 2,")?;
    writeln!(writer, "    \"<unk>\" => 3,")?;
    writeln!(writer, "}};")?;
    writeln!(writer, "static ID_TO_TOKEN: phf::Map<u32, &'static str> = phf::phf_map!{{")?;
    writeln!(writer, "    0 => \"<pad>\",")?;
    writeln!(writer, "    1 => \"<s>\",")?;
    writeln!(writer, "    2 => \"</s>\",")?;
    writeln!(writer, "    3 => \"<unk>\",")?;
    writeln!(writer, "}};")?;
    
    Ok(())
}

/// Extracts special token IDs from the tokenizer JSON
fn extract_special_tokens(
    tokenizer: &serde_json::Value,
    writer: &mut BufWriter<File>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:info=Extracting special tokens...");
    writeln!(writer, "// Special token IDs")?;
    
    // BOS token
    let bos_id = tokenizer["model"]["bos_token_id"]
        .as_u64()
        .or_else(|| tokenizer["model"]["bos_id"].as_u64())
        .unwrap_or(1);
    writeln!(writer, "pub const BOS_TOKEN_ID: u32 = {};", bos_id)?;
    
    // EOS token
    let eos_id = tokenizer["model"]["eos_token_id"]
        .as_u64()
        .or_else(|| tokenizer["model"]["eos_id"].as_u64())
        .unwrap_or(2);
    writeln!(writer, "pub const EOS_TOKEN_ID: u32 = {};", eos_id)?;
    
    // PAD token
    let pad_id = tokenizer["model"]["pad_token_id"]
        .as_u64()
        .or_else(|| tokenizer["model"]["pad_id"].as_u64())
        .unwrap_or(0);
    writeln!(writer, "pub const PAD_TOKEN_ID: u32 = {};", pad_id)?;
    
    // UNK token
    let unk_id = tokenizer["model"]["unk_token_id"]
        .as_u64()
        .or_else(|| tokenizer["model"]["unk_id"].as_u64())
        .unwrap_or(3);
    writeln!(writer, "pub const UNK_TOKEN_ID: u32 = {};", unk_id)?;
    writeln!(writer)?;
    
    // Write essential special token mapping
    writeln!(writer, "// ID to token mapping (minimal set for essential tokens)")?;
    writeln!(writer, "static ID_TO_TOKEN: phf::Map<u32, &'static str> = phf::phf_map! {{")?;
    writeln!(writer, "    0u32 => \"<|padding|>\",")?;
    writeln!(writer, "    1u32 => \"<|endoftext|>\",")?;
    writeln!(writer, "    2u32 => \"<|endoftext|>\",")?;
    writeln!(writer, "    3u32 => \"<|unknown|>\",")?;
    writeln!(writer, "}};")?;
    writeln!(writer)?;
    
    writeln!(writer, "pub fn get_id_to_token() -> &'static phf::Map<u32, &'static str> {{")?;
    writeln!(writer, "    &ID_TO_TOKEN")?;
    writeln!(writer, "}}")?;
    writeln!(writer)?;
    
    Ok(())
}

/// Extracts vocabulary from the tokenizer JSON
fn extract_vocabulary(
    tokenizer: &serde_json::Value,
    writer: &mut BufWriter<File>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:info=Building vocabulary map...");
    
    // Try to get the vocab object, first under "vocab" then under "tokens"
    let vocab_obj = if tokenizer["model"]["vocab"].is_object() {
        tokenizer["model"]["vocab"].as_object()
    } else {
        tokenizer["model"]["tokens"].as_object()
    };
    
    if let Some(vocab_obj) = vocab_obj {
        writeln!(writer, "// Vocabulary mapping")?;
        writeln!(writer, "static VOCAB: phf::Map<&'static str, u32> = ")?;
        
        let mut map = PhfMap::new();
        let mut count = 0;
        
        // Process the complete vocabulary - load all entries for correct tokenization
        for (token, id_value) in vocab_obj {
            if let Some(id) = id_value.as_u64() {
                count += 1;
                
                // Print progress for large vocabularies
                if count % 25000 == 0 {
                    println!("cargo:info=Processing vocabulary: {} entries so far", count);
                }
                
                // Escape any special characters in tokens
                let escaped_token = token.replace('\\', "\\\\").replace('"', "\\\"");
                map.entry(escaped_token, &format!("{}", id));
            }
        }
        
        println!("cargo:info=Vocabulary processed: {} total entries", count);
        
        write!(writer, "{}", map.build())?;
        writeln!(writer, ";")?;
        writeln!(writer)?;
    } else {
        // Fallback for no vocabulary
        println!("cargo:info=No vocabulary found in tokenizer, using minimal default");
        writeln!(writer, "static VOCAB: phf::Map<&'static str, u32> = phf::phf_map!{{")?;
        writeln!(writer, "    \"<pad>\" => 0,")?;
        writeln!(writer, "    \"<s>\" => 1,")?;
        writeln!(writer, "    \"</s>\" => 2,")?;
        writeln!(writer, "    \"<unk>\" => 3")?;
        writeln!(writer, "}};")?;
    }
    
    // Add the get_vocab function
    writeln!(writer, "pub fn get_vocab() -> &'static phf::Map<&'static str, u32> {{")?;
    writeln!(writer, "    &VOCAB")?;
    writeln!(writer, "}}")?;
    writeln!(writer)?;
    
    Ok(())
}

/// Extracts BPE merges from the tokenizer JSON
fn extract_merges(
    tokenizer: &serde_json::Value,
    writer: &mut BufWriter<File>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:info=Processing BPE merges...");
    
    if let Some(merges_arr) = tokenizer["model"]["merges"].as_array() {
        writeln!(writer, "// BPE merges ordered by priority (complete set for accurate tokenization)")?;
        writeln!(writer, "pub static MERGES: &[(&str, &str, usize)] = &[")?;
        
        let mut merge_count = 0;
        let mut skipped_count = 0;
        
        // Process all merges - print progress
        println!("cargo:info=Processing all {} BPE merges", merges_arr.len());
        
        for (rank, merge_value) in merges_arr.iter().enumerate() {
            // Print progress for large merge sets
            if merge_count > 0 && merge_count % 50000 == 0 {
                println!("cargo:info=Processed {} merges so far", merge_count);
            }
            
            let (first, second) = if let Some(merge_str) = merge_value.as_str() {
                // Format 1: "a b"
                let parts: Vec<&str> = merge_str.split(' ').collect();
                if parts.len() == 2 {
                    (parts[0], parts[1])
                } else {
                    skipped_count += 1;
                    continue;
                }
            } else if let Some(merge_array) = merge_value.as_array() {
                // Format 2: ["a", "b"]
                if merge_array.len() == 2 {
                    if let (Some(first), Some(second)) = (merge_array[0].as_str(), merge_array[1].as_str()) {
                        (first, second)
                    } else {
                        skipped_count += 1;
                        continue;
                    }
                } else {
                    skipped_count += 1;
                    continue;
                }
            } else {
                skipped_count += 1;
                continue;
            };
            
            // Escape any special characters
            let escaped_first = first.replace('\\', "\\\\").replace('"', "\\\"");
            let escaped_second = second.replace('\\', "\\\\").replace('"', "\\\"");
            
            writeln!(writer, "    (\"{}\", \"{}\", {}),", escaped_first, escaped_second, rank)?;
            merge_count += 1;
        }
        
        writeln!(writer, "];")?;
        
        if skipped_count > 0 {
            println!("cargo:info=Skipped {} invalid merges", skipped_count);
        }
    } else {
        println!("cargo:info=No merges found in tokenizer");
        writeln!(writer, "pub static MERGES: &[(&str, &str, usize)] = &[];")?;
    }
    
    Ok(())
}