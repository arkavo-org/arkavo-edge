//! `arkavo mcp proxy`: a permit-gated stdio MCP relay.
//!
//! Admits a `tools/call` only with a valid permit and proof-of-possession.
//! Configuration is flags only; the policy bundle hash pins which bundle
//! the permits must cite, and `--issuer-key` lists the issuer public keys
//! the dispatch gate trusts.

use arkavo_dispatch_gate::{DispatchGate, GateConfig, unix_now};
use arkavo_mcp_proxy::{McpProxy, PermitPolicy, ProxyConfig};
use arkavo_permit::{HashAlgorithm, PermitVerifier};
use std::sync::Arc;

const USAGE: &str = "usage: arkavo mcp proxy --policy-bundle-hash <64 hex> --issuer-key <hex> [--issuer-key <hex> ...] [--hash sha256|blake3] -- <upstream command> [args...]";

pub struct ProxyArgs {
    pub policy_bundle_hash: Vec<u8>,
    pub hash: HashAlgorithm,
    pub trusted_issuers: Vec<PermitVerifier>,
    pub command: String,
    pub args: Vec<String>,
}

// `execute` is the synchronous entry point invoked from `arkavo_cli::run`'s
// top-level command dispatch (never from inside an existing tokio runtime),
// so building a fresh `Runtime` here and blocking on it is safe.
#[allow(clippy::disallowed_methods)]
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let parsed = match parse(args) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            std::process::exit(2);
        }
    };
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(run(parsed))
}

async fn run(parsed: ProxyArgs) -> Result<(), Box<dyn std::error::Error>> {
    let gate = DispatchGate::new(GateConfig {
        policy_bundle_hash: parsed.policy_bundle_hash,
        hash: parsed.hash,
        clock: unix_now,
        trusted_issuers: parsed.trusted_issuers,
    });
    let config = ProxyConfig::new(parsed.command, parsed.args);
    let proxy = McpProxy::spawn(config, Arc::new(PermitPolicy::new(gate)))?;
    let stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let stdout = tokio::io::stdout();
    proxy.run(stdin, stdout).await?;
    Ok(())
}

fn parse(args: &[String]) -> Result<ProxyArgs, String> {
    if args.first().map(String::as_str) != Some("proxy") {
        return Err("unknown mcp subcommand; only `proxy` is available".into());
    }
    let mut policy_bundle_hash = None;
    let mut hash = HashAlgorithm::Sha256;
    let mut trusted_issuers = Vec::new();
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--policy-bundle-hash" => {
                let value = args
                    .get(index + 1)
                    .ok_or("--policy-bundle-hash needs a value")?;
                let bytes = decode_hex(value)?;
                if bytes.len() != 32 {
                    return Err("--policy-bundle-hash must be 32 bytes (64 hex)".into());
                }
                policy_bundle_hash = Some(bytes);
                index += 2;
            }
            "--issuer-key" => {
                let value = args.get(index + 1).ok_or("--issuer-key needs a value")?;
                trusted_issuers.push(decode_issuer_key(value)?);
                index += 2;
            }
            "--hash" => {
                let value = args.get(index + 1).ok_or("--hash needs a value")?;
                hash = HashAlgorithm::from_name(value)
                    .ok_or_else(|| format!("unknown hash {value}"))?;
                index += 2;
            }
            "--" => {
                let command = args
                    .get(index + 1)
                    .ok_or("missing upstream command after --")?
                    .clone();
                let rest = args[index + 2..].to_vec();
                let policy_bundle_hash =
                    policy_bundle_hash.ok_or("--policy-bundle-hash is required")?;
                if trusted_issuers.is_empty() {
                    return Err("at least one --issuer-key is required".into());
                }
                return Ok(ProxyArgs {
                    policy_bundle_hash,
                    hash,
                    trusted_issuers,
                    command,
                    args: rest,
                });
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Err("missing `-- <upstream command>`".into())
}

fn decode_hex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err("hex has odd length".into());
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

fn decode_issuer_key(text: &str) -> Result<PermitVerifier, String> {
    let bytes = decode_hex(text)?;
    PermitVerifier::from_public_key_bytes(&bytes).map_err(|e| format!("invalid --issuer-key: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|i| i.to_string()).collect()
    }

    fn ed25519_issuer_key_hex() -> String {
        hex::encode(
            arkavo_crypto::AgentKeypair::generate()
                .public_key()
                .to_bytes(),
        )
    }

    fn p256_issuer_key_hex() -> String {
        hex::encode(
            arkavo_crypto::P256SigningKeypair::generate()
                .public_key()
                .to_sec1_bytes(),
        )
    }

    #[test]
    fn parses_bundle_hash_hash_alg_and_upstream() {
        let hex = "07".repeat(32);
        let issuer_key = ed25519_issuer_key_hex();
        let parsed = parse(&s(&[
            "proxy",
            "--policy-bundle-hash",
            &hex,
            "--issuer-key",
            &issuer_key,
            "--hash",
            "blake3",
            "--",
            "python3",
            "srv.py",
            "--flag",
        ]))
        .unwrap();
        assert_eq!(parsed.policy_bundle_hash, vec![7u8; 32]);
        assert_eq!(parsed.hash, arkavo_permit::HashAlgorithm::Blake3);
        assert_eq!(parsed.command, "python3");
        assert_eq!(parsed.args, s(&["srv.py", "--flag"]));
        assert_eq!(parsed.trusted_issuers.len(), 1);
    }

    #[test]
    fn defaults_to_sha256() {
        let hex = "07".repeat(32);
        let issuer_key = ed25519_issuer_key_hex();
        let parsed = parse(&s(&[
            "proxy",
            "--policy-bundle-hash",
            &hex,
            "--issuer-key",
            &issuer_key,
            "--",
            "cmd",
        ]))
        .unwrap();
        assert_eq!(parsed.hash, arkavo_permit::HashAlgorithm::Sha256);
    }

    #[test]
    fn rejects_missing_upstream_and_bad_hash() {
        let hex = "07".repeat(32);
        assert!(parse(&s(&["proxy", "--policy-bundle-hash", &hex])).is_err());
        assert!(parse(&s(&["proxy", "--policy-bundle-hash", "zz", "--", "cmd"])).is_err());
        assert!(parse(&s(&["proxy", "--", "cmd"])).is_err());
        assert!(parse(&s(&["other"])).is_err());
    }

    #[test]
    fn accepts_ed25519_issuer_key() {
        let hex = "07".repeat(32);
        let issuer_key = ed25519_issuer_key_hex();
        let parsed = parse(&s(&[
            "proxy",
            "--policy-bundle-hash",
            &hex,
            "--issuer-key",
            &issuer_key,
            "--",
            "cmd",
        ]))
        .unwrap();
        assert_eq!(parsed.trusted_issuers.len(), 1);
    }

    #[test]
    fn accepts_p256_issuer_key() {
        let hex = "07".repeat(32);
        let issuer_key = p256_issuer_key_hex();
        assert_eq!(issuer_key.len(), 130);
        assert!(issuer_key.starts_with("04"));
        let parsed = parse(&s(&[
            "proxy",
            "--policy-bundle-hash",
            &hex,
            "--issuer-key",
            &issuer_key,
            "--",
            "cmd",
        ]))
        .unwrap();
        assert_eq!(parsed.trusted_issuers.len(), 1);
    }

    #[test]
    fn missing_issuer_key_is_rejected() {
        let hex = "07".repeat(32);
        assert!(parse(&s(&["proxy", "--policy-bundle-hash", &hex, "--", "cmd"])).is_err());
    }

    #[test]
    fn wrong_length_issuer_key_is_rejected() {
        let hex = "07".repeat(32);
        let bad_issuer_key = "ab".repeat(20);
        assert!(
            parse(&s(&[
                "proxy",
                "--policy-bundle-hash",
                &hex,
                "--issuer-key",
                &bad_issuer_key,
                "--",
                "cmd",
            ]))
            .is_err()
        );
    }
}
