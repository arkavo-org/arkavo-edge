//! Model identity attestation.
//!
//! Produces verifiable evidence that binds a running agent to the model
//! weights it loaded — content-addressed by BLAKE3 — together with the runtime
//! that executes them. The evidence maps directly onto SPIFFE/SPIRE workload
//! selectors, so a SPIRE workload attestor plugin registered as `model` can
//! emit exactly the selectors produced here.

use crate::gguf;
use crate::{AttestationError, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

/// The inference runtime that loads and executes a model's weights.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRuntime {
    LlamaCpp,
    Mlx,
    Other(String),
}

impl ModelRuntime {
    /// Stable lowercase label used in SPIRE selectors and SPIFFE paths.
    pub fn label(&self) -> &str {
        match self {
            ModelRuntime::LlamaCpp => "llama.cpp",
            ModelRuntime::Mlx => "mlx",
            ModelRuntime::Other(s) => s,
        }
    }

    /// Resolve a runtime from a user-supplied label, accepting common spellings.
    pub fn from_label(s: &str) -> Self {
        match s.to_ascii_lowercase().replace(['-', '_'], ".").as_str() {
            "llama.cpp" | "llamacpp" | "llama" => ModelRuntime::LlamaCpp,
            "mlx" => ModelRuntime::Mlx,
            _ => ModelRuntime::Other(s.to_string()),
        }
    }
}

/// On-disk weights container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFormat {
    Gguf,
    Safetensors,
    Unknown,
}

impl ModelFormat {
    fn label(self) -> &'static str {
        match self {
            ModelFormat::Gguf => "gguf",
            ModelFormat::Safetensors => "safetensors",
            ModelFormat::Unknown => "unknown",
        }
    }
}

/// Verifiable identity evidence for a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEvidence {
    /// Logical model identifier (GGUF `general.name`, else the file stem).
    pub model_id: String,
    /// Content address of the weights file: `blake3:<64 hex chars>`.
    pub weights_digest: String,
    /// Size of the weights file in bytes.
    pub weights_size: u64,
    /// On-disk weights format.
    pub format: ModelFormat,
    /// Runtime that executes the weights.
    pub runtime: ModelRuntime,
    /// Runtime build identifier (engine commit or version), when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_version: Option<String>,
    /// Model architecture from the GGUF `general.architecture` key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub architecture: Option<String>,
    /// Quantization scheme derived from the GGUF `general.file_type` key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantization: Option<String>,
    /// GGUF container version, when the file is GGUF.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gguf_version: Option<u32>,
    /// Tensor count declared in the GGUF header.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tensor_count: Option<u64>,
    /// Unix timestamp (seconds) when the evidence was produced.
    pub timestamp: i64,
}

impl ModelEvidence {
    /// SPIRE workload selectors derived from this evidence.
    ///
    /// Each entry is a `model:<key>:<value>` string — the exact form a SPIRE
    /// workload attestor plugin registered under the type `model` would emit.
    pub fn spire_selectors(&self) -> Vec<String> {
        let mut selectors = vec![
            format!("model:id:{}", self.model_id),
            format!("model:weights:{}", self.weights_digest),
            format!("model:format:{}", self.format.label()),
            format!("model:runtime:{}", self.runtime.label()),
        ];
        if let Some(v) = &self.runtime_version {
            selectors.push(format!("model:runtime-version:{v}"));
        }
        if let Some(a) = &self.architecture {
            selectors.push(format!("model:arch:{a}"));
        }
        if let Some(q) = &self.quantization {
            selectors.push(format!("model:quant:{q}"));
        }
        selectors
    }

    /// A SPIFFE ID proposal for an agent bound to this model.
    ///
    /// Shape: `spiffe://<trust_domain>/model/<arch>/<model-id>/<digest-prefix>`,
    /// each path segment normalized to a URL-safe form.
    pub fn spiffe_id(&self, trust_domain: &str) -> String {
        let arch = self.architecture.as_deref().unwrap_or("unknown");
        let digest_prefix: String = self
            .weights_digest
            .strip_prefix("blake3:")
            .unwrap_or(&self.weights_digest)
            .chars()
            .take(16)
            .collect();
        format!(
            "spiffe://{}/model/{}/{}/{}",
            trust_domain.trim_matches('/'),
            path_segment(arch),
            path_segment(&self.model_id),
            digest_prefix,
        )
    }
}

/// Attestor that produces [`ModelEvidence`] for a model.
pub trait ModelAttestor {
    fn attest(&self) -> Result<ModelEvidence>;
}

/// Attests a model from its on-disk weights file.
pub struct FileModelAttestor {
    weights_path: PathBuf,
    runtime: ModelRuntime,
    runtime_version: Option<String>,
}

impl FileModelAttestor {
    pub fn new(weights_path: impl Into<PathBuf>, runtime: ModelRuntime) -> Self {
        Self {
            weights_path: weights_path.into(),
            runtime,
            runtime_version: None,
        }
    }

    /// Record the runtime build identifier (engine commit or version string).
    pub fn with_runtime_version(mut self, version: impl Into<String>) -> Self {
        self.runtime_version = Some(version.into());
        self
    }
}

impl ModelAttestor for FileModelAttestor {
    fn attest(&self) -> Result<ModelEvidence> {
        let path = self.weights_path.as_path();
        let format = detect_format(path)?;
        let gguf_meta = if format == ModelFormat::Gguf {
            let file = File::open(path).map_err(|e| open_err(path, e))?;
            Some(gguf::parse(&mut BufReader::new(file))?)
        } else {
            None
        };
        let (weights_digest, weights_size) = digest_file(path)?;

        let model_id = gguf_meta
            .as_ref()
            .and_then(|m| m.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| file_stem(path));

        Ok(ModelEvidence {
            model_id,
            weights_digest,
            weights_size,
            format,
            runtime: self.runtime.clone(),
            runtime_version: self.runtime_version.clone(),
            architecture: gguf_meta.as_ref().and_then(|m| m.architecture.clone()),
            quantization: gguf_meta
                .as_ref()
                .and_then(gguf::GgufMetadata::quantization),
            gguf_version: gguf_meta.as_ref().map(|m| m.version),
            tensor_count: gguf_meta.as_ref().map(|m| m.tensor_count),
            timestamp: now_unix(),
        })
    }
}

fn detect_format(path: &Path) -> Result<ModelFormat> {
    let mut magic = [0u8; 4];
    let read = File::open(path)
        .and_then(|mut f| f.read(&mut magic))
        .map_err(|e| open_err(path, e))?;
    if read == 4 && &magic == b"GGUF" {
        return Ok(ModelFormat::Gguf);
    }
    if path.extension().and_then(|e| e.to_str()) == Some("safetensors") {
        return Ok(ModelFormat::Safetensors);
    }
    Ok(ModelFormat::Unknown)
}

/// Stream the weights file through BLAKE3, returning `(digest, size)`.
fn digest_file(path: &Path) -> Result<(String, u64)> {
    let file = File::open(path).map_err(|e| open_err(path, e))?;
    let size = file.metadata().map_err(|e| open_err(path, e))?.len();
    let mut reader = BufReader::new(file);
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = reader.read(&mut buf).map_err(|e| open_err(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok((format!("blake3:{}", hasher.finalize().to_hex()), size))
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| "model".to_string())
}

/// Normalize a string into a single URL-safe SPIFFE path segment.
fn path_segment(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.to_string()
    }
}

fn open_err(path: &Path, e: std::io::Error) -> AttestationError {
    AttestationError::Model(format!("{}: {e}", path.display()))
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn put_str(buf: &mut Vec<u8>, s: &str) {
        buf.extend_from_slice(&(s.len() as u64).to_le_bytes());
        buf.extend_from_slice(s.as_bytes());
    }

    /// A minimal but valid GGUF v3 header for `llama` / `Q4_K_M`.
    fn synthetic_gguf() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&0x4655_4747u32.to_le_bytes()); // "GGUF"
        b.extend_from_slice(&3u32.to_le_bytes()); // version
        b.extend_from_slice(&128u64.to_le_bytes()); // tensor_count
        b.extend_from_slice(&3u64.to_le_bytes()); // kv_count
        put_str(&mut b, "general.architecture");
        b.extend_from_slice(&8u32.to_le_bytes());
        put_str(&mut b, "llama");
        put_str(&mut b, "general.name");
        b.extend_from_slice(&8u32.to_le_bytes());
        put_str(&mut b, "Ministral 3 8B");
        put_str(&mut b, "general.file_type");
        b.extend_from_slice(&4u32.to_le_bytes());
        b.extend_from_slice(&15u32.to_le_bytes());
        b
    }

    struct TempFile(PathBuf);

    impl TempFile {
        fn new(suffix: &str, bytes: &[u8]) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let mut path = std::env::temp_dir();
            path.push(format!("arkavo-model-attest-{nanos}-{suffix}"));
            let mut file = File::create(&path).expect("create temp file");
            file.write_all(bytes).expect("write temp file");
            TempFile(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn runtime_label_and_parsing() {
        assert_eq!(ModelRuntime::LlamaCpp.label(), "llama.cpp");
        assert_eq!(ModelRuntime::Mlx.label(), "mlx");
        assert_eq!(
            ModelRuntime::from_label("llama-cpp"),
            ModelRuntime::LlamaCpp
        );
        assert_eq!(ModelRuntime::from_label("MLX"), ModelRuntime::Mlx);
        assert_eq!(
            ModelRuntime::from_label("vllm"),
            ModelRuntime::Other("vllm".to_string())
        );
        assert_eq!(ModelRuntime::Other("vllm".to_string()).label(), "vllm");
    }

    #[test]
    fn attests_gguf_model() {
        let tmp = TempFile::new("model.gguf", &synthetic_gguf());
        let evidence = FileModelAttestor::new(tmp.path(), ModelRuntime::LlamaCpp)
            .with_runtime_version("b9999")
            .attest()
            .expect("attest");

        assert_eq!(evidence.format, ModelFormat::Gguf);
        assert_eq!(evidence.model_id, "Ministral 3 8B");
        assert_eq!(evidence.architecture.as_deref(), Some("llama"));
        assert_eq!(evidence.quantization.as_deref(), Some("Q4_K_M"));
        assert_eq!(evidence.gguf_version, Some(3));
        assert_eq!(evidence.tensor_count, Some(128));
        assert_eq!(evidence.runtime_version.as_deref(), Some("b9999"));
        assert!(evidence.weights_digest.starts_with("blake3:"));
        assert_eq!(evidence.weights_digest.len(), "blake3:".len() + 64);
        assert_eq!(evidence.weights_size, synthetic_gguf().len() as u64);
    }

    #[test]
    fn digest_is_deterministic_and_content_addressed() {
        let tmp = TempFile::new("weights.bin", b"deterministic weights payload");
        let first = FileModelAttestor::new(tmp.path(), ModelRuntime::Mlx)
            .attest()
            .unwrap();
        let second = FileModelAttestor::new(tmp.path(), ModelRuntime::Mlx)
            .attest()
            .unwrap();
        assert_eq!(first.weights_digest, second.weights_digest);

        let other = TempFile::new("weights.bin", b"different weights payload");
        let third = FileModelAttestor::new(other.path(), ModelRuntime::Mlx)
            .attest()
            .unwrap();
        assert_ne!(first.weights_digest, third.weights_digest);
    }

    #[test]
    fn attests_safetensors_without_gguf_fields() {
        let tmp = TempFile::new("weights.safetensors", b"\x00\x01safetensors body");
        let evidence = FileModelAttestor::new(tmp.path(), ModelRuntime::Mlx)
            .attest()
            .unwrap();
        assert_eq!(evidence.format, ModelFormat::Safetensors);
        assert_eq!(evidence.runtime.label(), "mlx");
        assert!(evidence.architecture.is_none());
        assert!(evidence.gguf_version.is_none());
        // No GGUF name: model_id falls back to the file stem.
        assert!(evidence.model_id.ends_with("weights"));
    }

    #[test]
    fn unknown_format_for_unrecognized_file() {
        let tmp = TempFile::new("blob.bin", b"not a known model container");
        let evidence = FileModelAttestor::new(tmp.path(), ModelRuntime::LlamaCpp)
            .attest()
            .unwrap();
        assert_eq!(evidence.format, ModelFormat::Unknown);
    }

    #[test]
    fn missing_file_is_an_error() {
        let result =
            FileModelAttestor::new("/nonexistent/arkavo/model.gguf", ModelRuntime::LlamaCpp)
                .attest();
        assert!(result.is_err());
    }

    #[test]
    fn spire_selectors_cover_all_evidence_fields() {
        let tmp = TempFile::new("model.gguf", &synthetic_gguf());
        let evidence = FileModelAttestor::new(tmp.path(), ModelRuntime::LlamaCpp)
            .with_runtime_version("b9999")
            .attest()
            .unwrap();
        let selectors = evidence.spire_selectors();
        assert!(selectors.contains(&"model:id:Ministral 3 8B".to_string()));
        assert!(selectors.contains(&"model:format:gguf".to_string()));
        assert!(selectors.contains(&"model:runtime:llama.cpp".to_string()));
        assert!(selectors.contains(&"model:runtime-version:b9999".to_string()));
        assert!(selectors.contains(&"model:arch:llama".to_string()));
        assert!(selectors.contains(&"model:quant:Q4_K_M".to_string()));
        assert!(
            selectors
                .iter()
                .any(|s| s.starts_with("model:weights:blake3:"))
        );
    }

    #[test]
    fn spire_selectors_omit_absent_optional_fields() {
        let tmp = TempFile::new("weights.bin", b"opaque");
        let evidence = FileModelAttestor::new(tmp.path(), ModelRuntime::Mlx)
            .attest()
            .unwrap();
        let selectors = evidence.spire_selectors();
        assert!(!selectors.iter().any(|s| s.starts_with("model:arch:")));
        assert!(
            !selectors
                .iter()
                .any(|s| s.starts_with("model:runtime-version:"))
        );
    }

    #[test]
    fn spiffe_id_is_url_safe_and_normalized() {
        let tmp = TempFile::new("model.gguf", &synthetic_gguf());
        let evidence = FileModelAttestor::new(tmp.path(), ModelRuntime::LlamaCpp)
            .attest()
            .unwrap();
        let id = evidence.spiffe_id("agents.arkavo.com");
        assert!(id.starts_with("spiffe://agents.arkavo.com/model/llama/ministral-3-8b/"));
        assert!(!id.contains(' '));
        // trailing segment is the 16-char digest prefix
        assert_eq!(id.rsplit('/').next().unwrap().len(), 16);
    }

    #[test]
    fn evidence_serde_round_trip() {
        let tmp = TempFile::new("model.gguf", &synthetic_gguf());
        let evidence = FileModelAttestor::new(tmp.path(), ModelRuntime::LlamaCpp)
            .with_runtime_version("b9999")
            .attest()
            .unwrap();
        let json = serde_json::to_string(&evidence).unwrap();
        let restored: ModelEvidence = serde_json::from_str(&json).unwrap();
        assert_eq!(evidence, restored);
    }
}
