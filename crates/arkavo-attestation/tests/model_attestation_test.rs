use arkavo_attestation::{FileModelAttestor, ModelAttestor, ModelFormat, ModelRuntime};
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

/// Build a minimal but well-formed GGUF v3 header for `llama` / `Q4_K_M`.
fn synthetic_gguf() -> Vec<u8> {
    fn put_str(buf: &mut Vec<u8>, s: &str) {
        buf.extend_from_slice(&(s.len() as u64).to_le_bytes());
        buf.extend_from_slice(s.as_bytes());
    }
    let mut b = Vec::new();
    b.extend_from_slice(&0x4655_4747u32.to_le_bytes()); // "GGUF"
    b.extend_from_slice(&3u32.to_le_bytes()); // version
    b.extend_from_slice(&291u64.to_le_bytes()); // tensor_count
    b.extend_from_slice(&3u64.to_le_bytes()); // kv_count
    put_str(&mut b, "general.architecture");
    b.extend_from_slice(&8u32.to_le_bytes());
    put_str(&mut b, "llama");
    put_str(&mut b, "general.name");
    b.extend_from_slice(&8u32.to_le_bytes());
    put_str(&mut b, "Ministral 3 8B Instruct");
    put_str(&mut b, "general.file_type");
    b.extend_from_slice(&4u32.to_le_bytes());
    b.extend_from_slice(&15u32.to_le_bytes());
    b
}

struct TempFile(PathBuf);

impl TempFile {
    fn new(name: &str, bytes: &[u8]) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut path = std::env::temp_dir();
        path.push(format!("arkavo-model-attest-it-{nanos}-{name}"));
        File::create(&path)
            .and_then(|mut f| f.write_all(bytes))
            .expect("write temp model file");
        TempFile(path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn test_end_to_end_model_attestation_to_spire_selectors() {
    println!("\n=== End-to-End Model Attestation Test ===\n");

    let model = TempFile::new("ministral-3-8b-Q4_K_M.gguf", &synthetic_gguf());

    println!("Step 1: Construct a file-backed model attestor");
    let attestor = FileModelAttestor::new(&model.0, ModelRuntime::LlamaCpp)
        .with_runtime_version("llama.cpp-b4567");
    println!("✓ Attestor bound to {}", model.0.display());

    println!("\nStep 2: Attest the model");
    let evidence = attestor.attest().expect("attestation failed");
    println!("✓ Evidence produced");
    println!("  - model_id:        {}", evidence.model_id);
    println!("  - weights_digest:  {}", evidence.weights_digest);
    println!("  - weights_size:    {} bytes", evidence.weights_size);
    println!("  - runtime:         {}", evidence.runtime.label());
    println!("  - architecture:    {:?}", evidence.architecture);
    println!("  - quantization:    {:?}", evidence.quantization);

    assert_eq!(evidence.format, ModelFormat::Gguf);
    assert_eq!(evidence.model_id, "Ministral 3 8B Instruct");
    assert_eq!(evidence.architecture.as_deref(), Some("llama"));
    assert_eq!(evidence.quantization.as_deref(), Some("Q4_K_M"));
    assert_eq!(evidence.gguf_version, Some(3));
    assert_eq!(evidence.tensor_count, Some(291));
    assert!(evidence.weights_digest.starts_with("blake3:"));
    assert_eq!(evidence.weights_digest.len(), "blake3:".len() + 64);

    println!("\nStep 3: Content-addressing is deterministic");
    let again = FileModelAttestor::new(&model.0, ModelRuntime::LlamaCpp)
        .attest()
        .unwrap();
    assert_eq!(evidence.weights_digest, again.weights_digest);
    println!("✓ Re-attestation yields an identical weights digest");

    println!("\nStep 4: Derive SPIRE workload selectors");
    let selectors = evidence.spire_selectors();
    for s in &selectors {
        println!("  {s}");
    }
    assert!(selectors.contains(&"model:runtime:llama.cpp".to_string()));
    assert!(selectors.contains(&"model:arch:llama".to_string()));
    assert!(selectors.contains(&"model:quant:Q4_K_M".to_string()));
    assert!(
        selectors
            .iter()
            .any(|s| s.starts_with("model:weights:blake3:"))
    );

    println!("\nStep 5: Derive a SPIFFE ID");
    let spiffe_id = evidence.spiffe_id("agents.arkavo.com");
    println!("✓ {spiffe_id}");
    assert!(spiffe_id.starts_with("spiffe://agents.arkavo.com/model/llama/"));
    assert!(!spiffe_id.contains(' '));

    println!("\nStep 6: Evidence serializes to JSON");
    let json = serde_json::to_string_pretty(&evidence).expect("serialize");
    assert!(!json.is_empty());
    println!("{json}");

    println!("\n=== Model Attestation Flow Complete ===\n");
}

#[test]
fn test_mlx_safetensors_attestation() {
    println!("\n=== MLX / safetensors Attestation Test ===\n");

    let model = TempFile::new("model.safetensors", b"\x10\x00mlx safetensors body bytes");
    let evidence = FileModelAttestor::new(&model.0, ModelRuntime::Mlx)
        .with_runtime_version("mlx-0.18.0")
        .attest()
        .expect("attestation failed");

    assert_eq!(evidence.format, ModelFormat::Safetensors);
    assert_eq!(evidence.runtime, ModelRuntime::Mlx);
    // safetensors carries no GGUF key/value header.
    assert!(evidence.architecture.is_none());
    assert!(evidence.gguf_version.is_none());

    let selectors = evidence.spire_selectors();
    assert!(selectors.contains(&"model:runtime:mlx".to_string()));
    assert!(selectors.contains(&"model:format:safetensors".to_string()));
    assert!(!selectors.iter().any(|s| s.starts_with("model:arch:")));
    println!("✓ MLX evidence: {} selectors", selectors.len());

    println!("\n=== MLX Attestation Flow Complete ===\n");
}
