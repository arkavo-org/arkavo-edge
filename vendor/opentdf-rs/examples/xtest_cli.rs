//! Cross-SDK Test CLI for opentdf-rs
//!
//! A CLI matching the OpenTDFKit interface for symmetric cross-SDK testing.
//!
//! # Usage
//!
//! ```bash
//! # Encrypt
//! xtest_cli encrypt <input> <output> <format>
//!
//! # Decrypt
//! xtest_cli decrypt <input> <output> <format>
//!
//! # Check feature support
//! xtest_cli supports <feature>
//! ```
//!
//! # Supported Formats
//!
//! - `tdf` or `ztdf` - Standard ZIP-based TDF archive
//! - `json` - TDF-JSON inline format
//! - `cbor` - TDF-CBOR binary format
//!
//! # Environment Variables
//!
//! - `TDF_KAS_URL` - KAS endpoint URL
//! - `TDF_KAS_PUBLIC_KEY_PATH` - Path to KAS RSA/EC public key PEM
//! - `TDF_SYMMETRIC_KEY_PATH` - Path to symmetric key for decryption (hex encoded)
//! - `TDF_OUTPUT_SYMMETRIC_KEY_PATH` - Where to save symmetric key after encryption (hex encoded)
//!
//! # Example
//!
//! ```bash
//! export TDF_KAS_URL=http://localhost:9080/kas
//! export TDF_KAS_PUBLIC_KEY_PATH=/tmp/xtest/kas_public_rsa.pem
//! export TDF_OUTPUT_SYMMETRIC_KEY_PATH=/tmp/xtest/key.hex
//!
//! cargo run --example xtest_cli --features kas-client,cbor -- encrypt input.txt output.tdf tdf
//! cargo run --example xtest_cli --features kas-client,cbor -- decrypt output.tdf decrypted.txt tdf
//! ```

use clap::{Parser, Subcommand};
use std::path::PathBuf;

// ============================================================================
// CLI Definition
// ============================================================================

#[derive(Parser)]
#[command(name = "xtest_cli")]
#[command(about = "Cross-SDK Test CLI for opentdf-rs")]
#[command(version = "1.0.0")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Encrypt a file to TDF format
    Encrypt {
        /// Input plaintext file
        input: PathBuf,
        /// Output encrypted file
        output: PathBuf,
        /// Format: tdf, ztdf, json, cbor
        format: String,
    },
    /// Decrypt a TDF file
    Decrypt {
        /// Input encrypted file
        input: PathBuf,
        /// Output plaintext file
        output: PathBuf,
        /// Format: tdf, ztdf, json, cbor
        format: String,
    },
    /// Check if a feature is supported
    Supports {
        /// Feature name to check
        feature: String,
    },
}

// ============================================================================
// Configuration
// ============================================================================

#[derive(Debug)]
struct Config {
    kas_url: String,
    kas_public_key_path: Option<PathBuf>,
    symmetric_key_path: Option<PathBuf>,
    output_symmetric_key_path: Option<PathBuf>,
}

impl Config {
    fn from_env() -> Self {
        Self {
            kas_url: std::env::var("TDF_KAS_URL")
                .unwrap_or_else(|_| "http://localhost:9080/kas".to_string()),
            kas_public_key_path: std::env::var("TDF_KAS_PUBLIC_KEY_PATH")
                .ok()
                .map(PathBuf::from),
            symmetric_key_path: std::env::var("TDF_SYMMETRIC_KEY_PATH")
                .ok()
                .map(PathBuf::from),
            output_symmetric_key_path: std::env::var("TDF_OUTPUT_SYMMETRIC_KEY_PATH")
                .ok()
                .map(PathBuf::from),
        }
    }
}

// ============================================================================
// Format Detection
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
enum TdfFormat {
    Zip,  // Standard TDF (ZIP archive)
    Json, // TDF-JSON inline
    Cbor, // TDF-CBOR binary
}

impl TdfFormat {
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "tdf" | "ztdf" | "zip" => Ok(Self::Zip),
            "json" => Ok(Self::Json),
            "cbor" => Ok(Self::Cbor),
            _ => Err(format!("Unknown format: {}. Use tdf, json, or cbor.", s)),
        }
    }

    fn detect_from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 4 {
            return None;
        }

        // Check for ZIP magic bytes (PK..)
        if data[0] == 0x50 && data[1] == 0x4B {
            return Some(Self::Zip);
        }

        // Check for CBOR self-describe tag (D9 D9 F7)
        if data[0] == 0xD9 && data[1] == 0xD9 && data[2] == 0xF7 {
            return Some(Self::Cbor);
        }

        // Check for JSON (starts with { after optional whitespace)
        for &b in data.iter().take(100) {
            match b {
                b' ' | b'\t' | b'\n' | b'\r' => continue,
                b'{' => return Some(Self::Json),
                _ => break,
            }
        }

        None
    }
}

// ============================================================================
// Encryption
// ============================================================================

fn encrypt(
    input: &PathBuf,
    output: &PathBuf,
    format: TdfFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env();

    // Read input file
    let plaintext = std::fs::read(input)?;
    println!("Read {} bytes from {}", plaintext.len(), input.display());

    // Load KAS public key if provided
    let kas_public_key = match &config.kas_public_key_path {
        Some(path) => {
            let pem = std::fs::read_to_string(path)?;
            println!("Loaded KAS public key from {}", path.display());
            Some(pem)
        }
        None => None,
    };

    // Create policy
    let policy = opentdf::Policy::new(uuid::Uuid::new_v4().to_string(), vec![], vec![]);

    // Encrypt based on format
    let (encrypted, symmetric_key) = match format {
        TdfFormat::Zip => encrypt_zip(&plaintext, &config.kas_url, &policy)?,
        TdfFormat::Json => {
            let kas_key =
                kas_public_key.ok_or("TDF_KAS_PUBLIC_KEY_PATH required for JSON format")?;
            encrypt_json(&plaintext, &config.kas_url, &kas_key, &policy)?
        }
        TdfFormat::Cbor => {
            let kas_key =
                kas_public_key.ok_or("TDF_KAS_PUBLIC_KEY_PATH required for CBOR format")?;
            encrypt_cbor(&plaintext, &config.kas_url, &kas_key, &policy)?
        }
    };

    // Write encrypted output
    std::fs::write(output, &encrypted)?;
    println!("Wrote {} bytes to {}", encrypted.len(), output.display());

    // Save symmetric key if path provided
    if let Some(key_path) = &config.output_symmetric_key_path {
        let key_hex = hex::encode(&symmetric_key);
        std::fs::write(key_path, &key_hex)?;
        println!("Saved symmetric key to {}", key_path.display());
    }

    Ok(())
}

fn encrypt_zip(
    plaintext: &[u8],
    kas_url: &str,
    policy: &opentdf::Policy,
) -> Result<(Vec<u8>, Vec<u8>), Box<dyn std::error::Error>> {
    use opentdf::TdfArchiveMemoryBuilder;
    use opentdf::manifest::{
        IntegrityInformationExt, KeyAccessExt, Segment, TdfManifest, TdfManifestExt,
    };
    use opentdf_crypto::TdfEncryption;

    // Create encryption with random key
    let tdf_encryption = TdfEncryption::new()?;
    let symmetric_key = tdf_encryption.payload_key().to_vec();

    // Encrypt with segments
    let segment_size = 2 * 1024 * 1024; // 2MB segments
    let segmented = tdf_encryption.encrypt_with_segments(plaintext, segment_size)?;

    // Build manifest
    let mut manifest = TdfManifest::new("0.payload".to_string(), kas_url.to_string());
    manifest.encryption_information.method.algorithm = "AES-256-GCM".to_string();
    manifest.encryption_information.method.iv = String::new();

    // Set policy
    manifest.set_policy(policy)?;

    // Generate policy binding
    let policy_json = policy.to_json()?;
    manifest.encryption_information.key_access[0]
        .generate_policy_binding_raw(&policy_json, tdf_encryption.payload_key())?;

    // Add segment information
    for seg_info in &segmented.segment_info {
        manifest
            .encryption_information
            .integrity_information
            .segments
            .push(Segment {
                hash: seg_info.hash.clone(),
                segment_size: Some(seg_info.plaintext_size),
                encrypted_segment_size: Some(seg_info.encrypted_size),
            });
    }

    // Set segment defaults
    if let Some(first_seg) = segmented.segment_info.first() {
        manifest
            .encryption_information
            .integrity_information
            .segment_size_default = first_seg.plaintext_size;
        manifest
            .encryption_information
            .integrity_information
            .encrypted_segment_size_default = first_seg.encrypted_size;
    }

    // Generate root signature
    manifest
        .encryption_information
        .integrity_information
        .generate_root_signature(&segmented.gmac_tags, tdf_encryption.payload_key())?;

    // Build archive
    let mut builder = TdfArchiveMemoryBuilder::new();
    builder.add_entry_with_segments(&manifest, &segmented.segments, 0)?;
    let encrypted = builder.finish()?;

    Ok((encrypted, symmetric_key))
}

fn encrypt_json(
    plaintext: &[u8],
    kas_url: &str,
    kas_public_key: &str,
    policy: &opentdf::Policy,
) -> Result<(Vec<u8>, Vec<u8>), Box<dyn std::error::Error>> {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use opentdf::jsonrpc::{JsonPayload, TdfJson, TdfJsonManifest};
    use opentdf::manifest::{
        EncryptionInformation, EncryptionMethod, IntegrityInformation, IntegrityInformationExt,
        KeyAccess, RootSignature, Segment,
    };
    use opentdf_crypto::{TdfEncryption, calculate_policy_binding, wrap_key_with_ec};

    // Create encryption with random key
    let tdf_encryption = TdfEncryption::new()?;
    let symmetric_key = tdf_encryption.payload_key().to_vec();

    // Encrypt the data
    let encrypted_payload = tdf_encryption.encrypt(plaintext)?;

    // Create policy binding
    let policy_json = serde_json::to_string(policy)?;
    let policy_b64 = BASE64.encode(policy_json.as_bytes());
    let policy_hash = calculate_policy_binding(&policy_b64, &symmetric_key)?;

    // Wrap key with EC
    let ec_result = wrap_key_with_ec(kas_public_key, &symmetric_key)?;

    // Create key access object
    let key_access = KeyAccess {
        access_type: "wrapped".to_string(),
        url: kas_url.to_string(),
        kid: None,
        protocol: "kas".to_string(),
        wrapped_key: ec_result.wrapped_key,
        policy_binding: opentdf::manifest::PolicyBinding {
            alg: "HS256".to_string(),
            hash: policy_hash,
        },
        encrypted_metadata: None,
        schema_version: Some("1.0".to_string()),
        ephemeral_public_key: Some(ec_result.ephemeral_public_key),
    };

    // Extract only the 12-byte payload IV (not the combined 24-byte IV with key_iv)
    let iv_combined = BASE64.decode(&encrypted_payload.iv)?;
    let payload_iv = &iv_combined[..12]; // First 12 bytes is the payload IV
    let iv_b64 = BASE64.encode(payload_iv);

    // Get ciphertext bytes
    let ciphertext_bytes = BASE64.decode(&encrypted_payload.ciphertext)?;
    let gmac_tag = if ciphertext_bytes.len() >= 16 {
        ciphertext_bytes[ciphertext_bytes.len() - 16..].to_vec()
    } else {
        return Err("Ciphertext too short".into());
    };

    // Build segment with IV prepended (matches Swift format)
    // Swift format: IV (12 bytes) + ciphertext + tag
    let mut segment_data = payload_iv.to_vec();
    segment_data.extend_from_slice(&ciphertext_bytes);
    let encrypted_segment_size = segment_data.len() as u64;

    // Create integrity information
    let mut integrity_info = IntegrityInformation {
        root_signature: RootSignature {
            alg: "HS256".to_string(),
            sig: String::new(),
        },
        segment_hash_alg: "GMAC".to_string(),
        segments: vec![Segment {
            hash: BASE64.encode(&gmac_tag),
            segment_size: Some(plaintext.len() as u64),
            encrypted_segment_size: Some(encrypted_segment_size),
        }],
        segment_size_default: plaintext.len() as u64,
        encrypted_segment_size_default: encrypted_segment_size,
    };
    integrity_info.generate_root_signature(&[gmac_tag], &symmetric_key)?;

    // Create encryption information
    let encryption_info = EncryptionInformation {
        encryption_type: "split".to_string(),
        key_access: vec![key_access],
        method: EncryptionMethod {
            algorithm: "AES-256-GCM".to_string(),
            is_streamable: true,
            iv: iv_b64,
        },
        integrity_information: integrity_info,
        policy: policy_b64,
    };

    // Create TDF-JSON envelope
    let envelope = TdfJson {
        tdf: "json".to_string(),
        version: "1.0.0".to_string(),
        created: Some(chrono::Utc::now().to_rfc3339()),
        manifest: TdfJsonManifest {
            encryption_information: encryption_info,
            assertions: None,
        },
        payload: JsonPayload {
            payload_type: "inline".to_string(),
            protocol: "base64".to_string(),
            mime_type: Some("application/octet-stream".to_string()),
            is_encrypted: true,
            length: Some(encrypted_segment_size),
            value: BASE64.encode(&segment_data),
        },
    };

    let encrypted = serde_json::to_vec_pretty(&envelope)?;

    Ok((encrypted, symmetric_key))
}

fn encrypt_cbor(
    plaintext: &[u8],
    kas_url: &str,
    kas_public_key: &str,
    policy: &opentdf::Policy,
) -> Result<(Vec<u8>, Vec<u8>), Box<dyn std::error::Error>> {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use opentdf::manifest::{
        EncryptionInformation, EncryptionMethod, IntegrityInformation, IntegrityInformationExt,
        KeyAccess, RootSignature, Segment,
    };
    use opentdf::tdf_cbor::{CborPayload, TdfCbor};
    use opentdf_crypto::{TdfEncryption, calculate_policy_binding, wrap_key_with_ec};

    // Create encryption with random key
    let tdf_encryption = TdfEncryption::new()?;
    let symmetric_key = tdf_encryption.payload_key().to_vec();

    // Encrypt the data
    let encrypted_payload = tdf_encryption.encrypt(plaintext)?;

    // Create policy binding
    let policy_json = serde_json::to_string(policy)?;
    let policy_b64 = BASE64.encode(policy_json.as_bytes());
    let policy_hash = calculate_policy_binding(&policy_b64, &symmetric_key)?;

    // Wrap key with EC
    let ec_result = wrap_key_with_ec(kas_public_key, &symmetric_key)?;

    // Create key access object
    let key_access = KeyAccess {
        access_type: "wrapped".to_string(),
        url: kas_url.to_string(),
        kid: None,
        protocol: "kas".to_string(),
        wrapped_key: ec_result.wrapped_key,
        policy_binding: opentdf::manifest::PolicyBinding {
            alg: "HS256".to_string(),
            hash: policy_hash,
        },
        encrypted_metadata: None,
        schema_version: Some("1.0".to_string()),
        ephemeral_public_key: Some(ec_result.ephemeral_public_key),
    };

    // Extract only the 12-byte payload IV (not the combined 24-byte IV with key_iv)
    let iv_combined = BASE64.decode(&encrypted_payload.iv)?;
    let payload_iv = &iv_combined[..12]; // First 12 bytes is the payload IV
    let iv_b64 = BASE64.encode(payload_iv);

    // Get ciphertext bytes
    let ciphertext_bytes = BASE64.decode(&encrypted_payload.ciphertext)?;
    let gmac_tag = if ciphertext_bytes.len() >= 16 {
        ciphertext_bytes[ciphertext_bytes.len() - 16..].to_vec()
    } else {
        return Err("Ciphertext too short".into());
    };

    // Build segment with IV prepended (matches Swift format)
    // Swift format: IV (12 bytes) + ciphertext + tag
    let mut segment_data = payload_iv.to_vec();
    segment_data.extend_from_slice(&ciphertext_bytes);
    let encrypted_segment_size = segment_data.len() as u64;

    // Create integrity information
    let mut integrity_info = IntegrityInformation {
        root_signature: RootSignature {
            alg: "HS256".to_string(),
            sig: String::new(),
        },
        segment_hash_alg: "GMAC".to_string(),
        segments: vec![Segment {
            hash: BASE64.encode(&gmac_tag),
            segment_size: Some(plaintext.len() as u64),
            encrypted_segment_size: Some(encrypted_segment_size),
        }],
        segment_size_default: plaintext.len() as u64,
        encrypted_segment_size_default: encrypted_segment_size,
    };
    integrity_info.generate_root_signature(&[gmac_tag], &symmetric_key)?;

    // Create encryption information
    let encryption_info = EncryptionInformation {
        encryption_type: "split".to_string(),
        key_access: vec![key_access],
        method: EncryptionMethod {
            algorithm: "AES-256-GCM".to_string(),
            is_streamable: true,
            iv: iv_b64,
        },
        integrity_information: integrity_info,
        policy: policy_b64,
    };

    // Create TDF-CBOR envelope
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .ok();

    let envelope = TdfCbor {
        tdf: "cbor".to_string(),
        version: [1, 0, 0],
        created: timestamp,
        manifest: opentdf::tdf_cbor::TdfCborManifest {
            encryption_information: encryption_info,
            assertions: None,
        },
        payload: CborPayload {
            payload_type: "inline".to_string(),
            protocol: "binary".to_string(),
            mime_type: Some("application/octet-stream".to_string()),
            is_encrypted: true,
            value: segment_data,
        },
    };

    let encrypted = envelope.to_bytes()?;

    Ok((encrypted, symmetric_key))
}

// ============================================================================
// Decryption
// ============================================================================

fn decrypt(
    input: &PathBuf,
    output: &PathBuf,
    format_hint: TdfFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env();

    // Read encrypted file
    let encrypted = std::fs::read(input)?;
    println!("Read {} bytes from {}", encrypted.len(), input.display());

    // Auto-detect format if possible
    let format = TdfFormat::detect_from_bytes(&encrypted).unwrap_or(format_hint);
    println!("Detected format: {:?}", format);

    // Load symmetric key for offline decryption
    let symmetric_key = match &config.symmetric_key_path {
        Some(path) => {
            let key_hex = std::fs::read_to_string(path)?;
            let key = hex::decode(key_hex.trim())?;
            println!("Loaded symmetric key from {}", path.display());
            Some(key)
        }
        None => None,
    };

    // Decrypt based on format
    let plaintext = match format {
        TdfFormat::Zip => decrypt_zip(&encrypted, symmetric_key.as_deref())?,
        TdfFormat::Json => {
            let key = symmetric_key.ok_or("TDF_SYMMETRIC_KEY_PATH required for JSON decryption")?;
            decrypt_json(&encrypted, &key)?
        }
        TdfFormat::Cbor => {
            let key = symmetric_key.ok_or("TDF_SYMMETRIC_KEY_PATH required for CBOR decryption")?;
            decrypt_cbor(&encrypted, &key)?
        }
    };

    // Write decrypted output
    std::fs::write(output, &plaintext)?;
    println!("Wrote {} bytes to {}", plaintext.len(), output.display());

    Ok(())
}

fn decrypt_zip(
    encrypted: &[u8],
    symmetric_key: Option<&[u8]>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use aes_gcm::{
        Aes256Gcm, Nonce,
        aead::{Aead, KeyInit},
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use opentdf::TdfArchive;

    let cursor = std::io::Cursor::new(encrypted.to_vec());
    let mut archive = TdfArchive::new(cursor)?;
    let entry = archive.by_index()?;

    // For offline decryption, we need the symmetric key
    let key = symmetric_key.ok_or("TDF_SYMMETRIC_KEY_PATH required for offline ZIP decryption")?;

    // Check if segmented (modern format) or legacy (single block)
    let segments = &entry
        .manifest
        .encryption_information
        .integrity_information
        .segments;

    if !segments.is_empty() {
        // Modern segmented format - use TdfEncryption for decryption
        use opentdf_crypto::TdfEncryption;

        let tdf_encryption =
            TdfEncryption::with_payload_key(key).map_err(|e| format!("Invalid key: {:?}", e))?;

        let segment_sizes: Vec<(u64, u64)> = segments
            .iter()
            .map(|s| {
                (
                    s.segment_size.unwrap_or(0),
                    s.encrypted_segment_size.unwrap_or(0),
                )
            })
            .collect();

        let (plaintext, _) = tdf_encryption
            .decrypt_with_segments(&entry.payload, &segment_sizes)
            .map_err(|e| format!("Segment decryption failed: {:?}", e))?;

        Ok(plaintext)
    } else {
        // Legacy single-block format
        let iv_b64 = &entry.manifest.encryption_information.method.iv;
        let iv = BASE64.decode(iv_b64)?;

        #[allow(deprecated)]
        let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| "Invalid key length")?;
        #[allow(deprecated)]
        let nonce = Nonce::from_slice(&iv);

        #[allow(deprecated)]
        let plaintext = cipher
            .decrypt(nonce, entry.payload.as_ref())
            .map_err(|e| format!("Decryption failed: {}", e))?;

        Ok(plaintext)
    }
}

fn decrypt_json(
    encrypted: &[u8],
    symmetric_key: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use opentdf::jsonrpc::TdfJson;

    let envelope: TdfJson = serde_json::from_slice(encrypted)?;

    // Decode the payload
    let payload_bytes = BASE64.decode(&envelope.payload.value)?;

    // Check if payload has IV prepended (Swift format: 12 IV + ciphertext + 16 tag)
    // by comparing payload length with encrypted_segment_size_default
    let segments = &envelope
        .manifest
        .encryption_information
        .integrity_information
        .segments;
    let expected_size = if let Some(seg) = segments.first() {
        seg.encrypted_segment_size.unwrap_or(0) as usize
    } else {
        envelope
            .manifest
            .encryption_information
            .integrity_information
            .encrypted_segment_size_default as usize
    };

    // If payload is larger than expected encrypted size, IV is prepended
    let (iv, ciphertext) = if payload_bytes.len() > expected_size || payload_bytes.len() > 12 + 16 {
        // Check if first 12 bytes could be IV (heuristic: payload starts with IV)
        // Try IV-prepended format first (Swift format)
        (&payload_bytes[..12], &payload_bytes[12..])
    } else {
        // Use IV from manifest (legacy format)
        let iv = BASE64.decode(&envelope.manifest.encryption_information.method.iv)?;
        return decrypt_aes_gcm(symmetric_key, &iv, &payload_bytes);
    };

    decrypt_aes_gcm(symmetric_key, iv, ciphertext)
}

fn decrypt_aes_gcm(
    key: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use aes_gcm::{
        Aes256Gcm, Nonce,
        aead::{Aead, KeyInit},
    };

    #[allow(deprecated)]
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| "Invalid key length")?;
    #[allow(deprecated)]
    let nonce = Nonce::from_slice(iv);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("AES-GCM decryption failed: {}", e).into())
}

fn decrypt_cbor(
    encrypted: &[u8],
    symmetric_key: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use opentdf::tdf_cbor::TdfCbor;

    let envelope = TdfCbor::from_bytes(encrypted)?;

    // Get the raw payload bytes (CBOR uses binary protocol, not base64)
    let payload_bytes = &envelope.payload.value;

    // Check expected segment size to determine format
    let segments = &envelope
        .manifest
        .encryption_information
        .integrity_information
        .segments;
    let expected_size = if let Some(seg) = segments.first() {
        seg.encrypted_segment_size.unwrap_or(0) as usize
    } else {
        envelope
            .manifest
            .encryption_information
            .integrity_information
            .encrypted_segment_size_default as usize
    };

    // If payload is larger than expected encrypted size, IV is prepended
    let (iv, ciphertext) = if payload_bytes.len() > expected_size || payload_bytes.len() > 12 + 16 {
        // IV-prepended format (Swift format)
        (&payload_bytes[..12], &payload_bytes[12..])
    } else {
        // Use IV from manifest (legacy format)
        let iv = BASE64.decode(&envelope.manifest.encryption_information.method.iv)?;
        return decrypt_aes_gcm(symmetric_key, &iv, payload_bytes);
    };

    decrypt_aes_gcm(symmetric_key, iv, ciphertext)
}

// ============================================================================
// Feature Support Check
// ============================================================================

fn supports(feature: &str) -> bool {
    match feature.to_lowercase().as_str() {
        // Core formats
        "tdf" | "ztdf" | "zip" => true,
        "json" | "tdf-json" => true,
        "cbor" | "tdf-cbor" => true,

        // Encryption features
        "aes-256-gcm" | "aes256gcm" => true,
        "rsa-2048" | "rsa" => true,
        "ec" | "ecdh" | "p256" => true,

        // Policy features
        "policy" | "abac" => true,
        "hmac-policy-binding" => true,

        // KAS features
        "kas" | "kas-client" => true,
        "kas-rewrap" => true,

        // Not yet supported
        "nano" | "nanotdf" => false, // Deferred - requires EC key wrapping in mock KAS
        "streaming" => false,
        "multi-segment" => false,
        "assertions" => false,

        _ => false,
    }
}

// ============================================================================
// Main
// ============================================================================

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Encrypt {
            input,
            output,
            format,
        } => {
            let fmt = match TdfFormat::from_str(&format) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            };
            encrypt(&input, &output, fmt)
        }
        Commands::Decrypt {
            input,
            output,
            format,
        } => {
            let fmt = match TdfFormat::from_str(&format) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            };
            decrypt(&input, &output, fmt)
        }
        Commands::Supports { feature } => {
            if supports(&feature) {
                println!("Feature '{}' is supported", feature);
                std::process::exit(0);
            } else {
                println!("Feature '{}' is NOT supported", feature);
                std::process::exit(1);
            }
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_detection() {
        // ZIP magic bytes
        assert_eq!(
            TdfFormat::detect_from_bytes(&[0x50, 0x4B, 0x03, 0x04]),
            Some(TdfFormat::Zip)
        );

        // CBOR self-describe tag
        assert_eq!(
            TdfFormat::detect_from_bytes(&[0xD9, 0xD9, 0xF7, 0xA5]),
            Some(TdfFormat::Cbor)
        );

        // JSON
        assert_eq!(
            TdfFormat::detect_from_bytes(b"{\"tdf\":\"json\"}"),
            Some(TdfFormat::Json)
        );
        assert_eq!(
            TdfFormat::detect_from_bytes(b"  \n{\"tdf\":\"json\"}"),
            Some(TdfFormat::Json)
        );
    }

    #[test]
    fn test_format_from_str() {
        assert_eq!(TdfFormat::from_str("tdf").unwrap(), TdfFormat::Zip);
        assert_eq!(TdfFormat::from_str("ztdf").unwrap(), TdfFormat::Zip);
        assert_eq!(TdfFormat::from_str("json").unwrap(), TdfFormat::Json);
        assert_eq!(TdfFormat::from_str("cbor").unwrap(), TdfFormat::Cbor);
        assert!(TdfFormat::from_str("unknown").is_err());
    }

    #[test]
    fn test_supports() {
        assert!(supports("tdf"));
        assert!(supports("json"));
        assert!(supports("cbor"));
        assert!(supports("kas"));
        assert!(!supports("nano"));
        assert!(!supports("unknown-feature"));
    }
}
