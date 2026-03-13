//! PKCE (Proof Key for Code Exchange) and authorization URL helpers

use super::{AuthResult, OAuthClient, OAuthError};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use std::io::{BufRead, Write};

/// PKCE code challenge data
#[derive(Debug, Clone)]
pub(super) struct PkceChallenge {
    /// Code verifier (random string)
    pub(super) verifier: String,
    /// Code challenge (SHA-256 hash of verifier, base64url encoded)
    pub(super) challenge: String,
}

impl PkceChallenge {
    /// Generate a new PKCE challenge using cryptographically secure randomness
    pub(super) fn generate() -> Self {
        // 32 bytes of CSPRNG entropy per RFC 7636
        let mut random_bytes = [0u8; 32];
        getrandom::getrandom(&mut random_bytes).expect("getrandom failed");

        // Base64url encode for verifier (gives us 43 chars from 32 bytes)
        let verifier = URL_SAFE_NO_PAD.encode(random_bytes);

        // Create code challenge: BASE64URL(SHA256(verifier))
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let hash = hasher.finalize();
        let challenge = URL_SAFE_NO_PAD.encode(hash);

        Self {
            verifier,
            challenge,
        }
    }
}

impl OAuthClient {
    /// Build the authorization URL with PKCE challenge
    pub(super) fn build_auth_url(&self, code_challenge: &str) -> String {
        let state = Self::generate_state();

        // Use proper URL encoding
        let params = [
            ("client_id", self.config.client_id.as_str()),
            ("response_type", "code"),
            ("redirect_uri", self.config.redirect_uri.as_str()),
            ("scope", self.config.scopes.as_str()),
            ("code_challenge", code_challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
            ("code", "true"),
        ];

        let query = params
            .iter()
            .map(|(k, v)| format!("{k}={}", urlencoding(v)))
            .collect::<Vec<_>>()
            .join("&");

        format!("{}?{query}", self.config.auth_url)
    }

    /// Generate a state parameter using cryptographically secure randomness
    pub(super) fn generate_state() -> String {
        let mut random_bytes = [0u8; 24];
        getrandom::getrandom(&mut random_bytes).expect("getrandom failed");
        URL_SAFE_NO_PAD.encode(random_bytes) // 24 bytes = 32 chars in base64
    }

    /// Prompt user for authorization code
    pub(super) fn prompt_for_code() -> AuthResult<(String, Option<String>)> {
        print!("Enter authorization code (or 'cancel' to abort): ");
        std::io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().lock().read_line(&mut input)?;

        let input = input.trim();

        // The callback page displays "code#state" format - extract both parts
        let (code, state) = if let Some(hash_pos) = input.find('#') {
            let code = &input[..hash_pos];
            let state = &input[hash_pos + 1..];
            (code.to_string(), Some(state.to_string()))
        } else {
            (input.to_string(), None)
        };

        Ok((code, state))
    }

    /// Open URL in default browser
    pub(super) fn open_browser(url: &str) -> AuthResult<()> {
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .arg(url)
                .spawn()
                .map_err(|e| OAuthError::BrowserOpen(e.to_string()))?;
        }

        #[cfg(target_os = "linux")]
        {
            std::process::Command::new("xdg-open")
                .arg(url)
                .spawn()
                .map_err(|e| OAuthError::BrowserOpen(e.to_string()))?;
        }

        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("cmd")
                .args(["/C", "start", "", url])
                .spawn()
                .map_err(|e| OAuthError::BrowserOpen(e.to_string()))?;
        }

        Ok(())
    }
}

/// URL encode a string for OAuth parameters.
/// Preserves unreserved characters per RFC 3986.
pub(super) fn urlencoding(s: &str) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                write!(result, "%{byte:02X}").unwrap();
            }
        }
    }
    result
}
