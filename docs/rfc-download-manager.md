# RFC: Model Download Manager for Local LLM Support

## Overview
Minimal download manager for acquiring GGUF models with integrity verification and license compliance.

## Scope

### MUST Have (P0 - Beta Blocker)
- [ ] Download GGUF models from Hugging Face Hub
- [ ] SHA-256 verification against known checksums
- [ ] Write `Notice.txt` alongside downloaded models (license compliance)
- [ ] Progress indication during download
- [ ] Atomic downloads (temp file → rename on success)

### SHOULD Have (P1 - Post-Beta)
- [ ] Resume interrupted downloads
- [ ] Respect `HTTPS_PROXY`/`HTTP_PROXY` environment variables
- [ ] Parallel chunk downloads for large files
- [ ] Cache management (list/prune old models)

### MAY Have (P2 - Future)
- [ ] Mirror support (fallback URLs)
- [ ] Bandwidth throttling
- [ ] P2P distribution support

## Implementation Plan (<300 LOC)

### 1. Core Types
```rust
pub struct ModelDownloader {
    cache_dir: PathBuf,
    client: reqwest::Client,
}

pub struct ModelSpec {
    pub repo_id: String,      // e.g. "TinyLlama/TinyLlama-1.1B-Chat-v1.0-GGUF"
    pub filename: String,     // e.g. "tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf"
    pub sha256: String,       // Expected checksum
    pub license_text: String, // Content for Notice.txt
}
```

### 2. API Design
```rust
impl ModelDownloader {
    pub async fn download(&self, spec: &ModelSpec) -> Result<PathBuf>;
    pub fn get_model_path(&self, spec: &ModelSpec) -> PathBuf;
    pub fn is_downloaded(&self, spec: &ModelSpec) -> bool;
}
```

### 3. Directory Structure
```
~/.cache/arkavo/models/
├── tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf
├── tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf.sha256
└── tinyllama-1.1b-chat-v1.0.Q4_K_M.Notice.txt
```

### 4. CLI Integration
```bash
# Download during first use
arkavo chat --model tinyllama

# Or explicit download
arkavo model download tinyllama
arkavo model list
arkavo model remove tinyllama
```

## Dependencies
- `hf-hub = "0.3"` - Official Hugging Face Hub client
- `sha2 = "0.10"` - SHA-256 verification
- `indicatif = "0.17"` - Progress bars

## Security Considerations
- Verify SHA-256 before moving to final location
- Use OS temp directory for partial downloads
- Validate repo_id format to prevent path traversal
- HTTPS only (no HTTP fallback)

## Example Usage
```rust
let downloader = ModelDownloader::new()?;
let spec = ModelSpec {
    repo_id: "TinyLlama/TinyLlama-1.1B-Chat-v1.0-GGUF".into(),
    filename: "tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf".into(),
    sha256: "abc123...".into(),
    license_text: include_str!("../licenses/tinyllama.txt"),
};

let model_path = downloader.download(&spec).await?;
```

## Success Metrics
- Download completes in <5 minutes on 50 Mbps connection
- Corrupted downloads detected 100% of the time
- Zero security vulnerabilities
- Works behind corporate proxies