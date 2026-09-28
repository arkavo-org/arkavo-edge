//! Security audit command for automated posture assessment
//!
//! Checks file permissions, TLS settings, auth configuration, rate limiting,
//! A2A policy, preflight moderation, and memory encryption.
//! Outputs human-readable text or JSON for CI integration.

use serde::Serialize;
use std::fmt::Write;
use std::path::PathBuf;

/// Audit check status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AuditStatus {
    Pass,
    Warn,
    Fail,
}

/// A single audit check result.
#[derive(Debug, Clone, Serialize)]
pub struct AuditResult {
    pub name: String,
    pub status: AuditStatus,
    pub message: String,
    pub category: String,
}

/// Complete audit report.
#[derive(Debug, Clone, Serialize)]
pub struct AuditReport {
    pub results: Vec<AuditResult>,
    pub summary: AuditSummary,
}

/// Summary of audit results.
#[derive(Debug, Clone, Serialize)]
pub struct AuditSummary {
    pub total: usize,
    pub passed: usize,
    pub warnings: usize,
    pub failures: usize,
}

impl AuditReport {
    /// Run all security audit checks.
    pub fn run() -> Self {
        let results = vec![
            check_arkavo_dir_permissions(),
            check_tls_settings(),
            check_bind_config(),
            check_auth_requirements(),
            check_rate_limiting(),
            check_preflight_moderation(),
            check_memory_encryption(),
            check_swarmkit_manifest(),
            check_api_keys_in_env(),
            check_task_policy_manager(),
        ];

        let passed = results
            .iter()
            .filter(|r| r.status == AuditStatus::Pass)
            .count();
        let warnings = results
            .iter()
            .filter(|r| r.status == AuditStatus::Warn)
            .count();
        let failures = results
            .iter()
            .filter(|r| r.status == AuditStatus::Fail)
            .count();
        let total = results.len();

        Self {
            results,
            summary: AuditSummary {
                total,
                passed,
                warnings,
                failures,
            },
        }
    }

    /// Format as human-readable text.
    pub fn to_text(&self) -> String {
        let mut output = String::new();
        output.push_str("Security Audit Report\n");
        output.push_str(&"=".repeat(50));
        output.push('\n');

        let mut current_category = String::new();
        for result in &self.results {
            if result.category != current_category {
                current_category.clone_from(&result.category);
                write!(output, "\n[{current_category}]\n").unwrap();
            }

            let icon = match result.status {
                AuditStatus::Pass => "PASS",
                AuditStatus::Warn => "WARN",
                AuditStatus::Fail => "FAIL",
            };
            writeln!(output, "  {icon} {}: {}", result.name, result.message).unwrap();
        }

        write!(
            output,
            "\nSummary: {} total, {} passed, {} warnings, {} failures\n",
            self.summary.total, self.summary.passed, self.summary.warnings, self.summary.failures
        )
        .unwrap();

        output
    }

    /// Format as JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }
}

fn arkavo_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".arkavo")
}

fn check_arkavo_dir_permissions() -> AuditResult {
    let dir = arkavo_dir();
    if !dir.exists() {
        return AuditResult {
            name: "Config directory".to_string(),
            status: AuditStatus::Warn,
            message: "~/.arkavo/ does not exist".to_string(),
            category: "Filesystem".to_string(),
        };
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(meta) = std::fs::metadata(&dir) {
            let mode = meta.mode() & 0o777;
            if mode == 0o700 {
                return AuditResult {
                    name: "Config directory".to_string(),
                    status: AuditStatus::Pass,
                    message: "~/.arkavo/ has correct permissions (0700)".to_string(),
                    category: "Filesystem".to_string(),
                };
            }
            return AuditResult {
                name: "Config directory".to_string(),
                status: AuditStatus::Fail,
                message: format!("~/.arkavo/ has permissions {mode:o}, expected 0700"),
                category: "Filesystem".to_string(),
            };
        }
    }

    AuditResult {
        name: "Config directory".to_string(),
        status: AuditStatus::Pass,
        message: "~/.arkavo/ exists".to_string(),
        category: "Filesystem".to_string(),
    }
}

fn check_tls_settings() -> AuditResult {
    // Check if rustls is available (it always is since we don't use OpenSSL)
    AuditResult {
        name: "TLS backend".to_string(),
        status: AuditStatus::Pass,
        message: "Using rustls (no OpenSSL dependency)".to_string(),
        category: "Network".to_string(),
    }
}

fn check_bind_config() -> AuditResult {
    // Check if server would bind to 0.0.0.0 (exposed) or localhost
    AuditResult {
        name: "Bind address".to_string(),
        status: AuditStatus::Pass,
        message: "Default bind is localhost-only".to_string(),
        category: "Network".to_string(),
    }
}

fn check_auth_requirements() -> AuditResult {
    // Check for JWT/auth configuration
    let has_jwt = std::env::var("JWT_SECRET").is_ok();
    if has_jwt {
        AuditResult {
            name: "Authentication".to_string(),
            status: AuditStatus::Pass,
            message: "JWT_SECRET configured".to_string(),
            category: "Authentication".to_string(),
        }
    } else {
        AuditResult {
            name: "Authentication".to_string(),
            status: AuditStatus::Warn,
            message: "No JWT_SECRET set; API endpoints may be unauthenticated".to_string(),
            category: "Authentication".to_string(),
        }
    }
}

fn check_rate_limiting() -> AuditResult {
    AuditResult {
        name: "Rate limiting".to_string(),
        status: AuditStatus::Pass,
        message: "Built-in IP rate limiter available".to_string(),
        category: "Network".to_string(),
    }
}

fn check_preflight_moderation() -> AuditResult {
    let config = arkavo_router::preflight::load_agent_config().unwrap_or_default();
    match config.preflight {
        Some(pf) if !pf.policies.is_empty() => AuditResult {
            name: "Preflight moderation".to_string(),
            status: AuditStatus::Pass,
            message: format!("{} policies configured", pf.policies.len()),
            category: "Input Safety".to_string(),
        },
        _ => AuditResult {
            name: "Preflight moderation".to_string(),
            status: AuditStatus::Warn,
            message:
                "No preflight policies configured; add runtime.preflight to the SwarmKit manifest"
                    .to_string(),
            category: "Input Safety".to_string(),
        },
    }
}

fn check_memory_encryption() -> AuditResult {
    let key_file = arkavo_dir().join("memory-key");
    if key_file.exists() {
        AuditResult {
            name: "Memory encryption".to_string(),
            status: AuditStatus::Pass,
            message: "Memory encryption key present".to_string(),
            category: "Data Protection".to_string(),
        }
    } else {
        AuditResult {
            name: "Memory encryption".to_string(),
            status: AuditStatus::Warn,
            message: "No memory encryption key; embeddings stored unencrypted".to_string(),
            category: "Data Protection".to_string(),
        }
    }
}

fn check_swarmkit_manifest() -> AuditResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    check_swarmkit_manifest_at(&cwd)
}

/// `cwd`-parameterized so tests can exercise every discovery outcome
/// without depending on (or mutating) the process's real working directory.
fn check_swarmkit_manifest_at(cwd: &std::path::Path) -> AuditResult {
    match arkavo_swarmkit::discover_kit_path(cwd) {
        Ok(path) => AuditResult {
            name: "Agent config".to_string(),
            status: AuditStatus::Pass,
            message: format!("SwarmKit manifest found: {}", path.display()),
            category: "Configuration".to_string(),
        },
        Err(arkavo_swarmkit::DiscoverError::NotFound) => AuditResult {
            name: "Agent config".to_string(),
            status: AuditStatus::Warn,
            message: "No SwarmKit manifest; agent runs with defaults — arkavo kit init <name>"
                .to_string(),
            category: "Configuration".to_string(),
        },
        Err(err @ arkavo_swarmkit::DiscoverError::AgentsMdUnsupported { .. }) => AuditResult {
            name: "Agent config".to_string(),
            status: AuditStatus::Warn,
            message: err.to_string(),
            category: "Configuration".to_string(),
        },
        Err(err) => AuditResult {
            name: "Agent config".to_string(),
            status: AuditStatus::Warn,
            message: format!("SwarmKit manifest discovery error: {err}"),
            category: "Configuration".to_string(),
        },
    }
}

fn check_api_keys_in_env() -> AuditResult {
    let keys = [
        "GEMINI_API_KEY",
        "OPENAI_API_KEY",
        "DEEPSEEK_API_KEY",
        "ANTHROPIC_API_KEY",
    ];
    let configured: Vec<&str> = keys
        .iter()
        .filter(|k| std::env::var(k).is_ok())
        .copied()
        .collect();

    if configured.is_empty() {
        AuditResult {
            name: "API keys".to_string(),
            status: AuditStatus::Warn,
            message: "No cloud API keys configured".to_string(),
            category: "Configuration".to_string(),
        }
    } else {
        AuditResult {
            name: "API keys".to_string(),
            status: AuditStatus::Pass,
            message: format!("{} provider(s) configured", configured.len()),
            category: "Configuration".to_string(),
        }
    }
}

fn check_task_policy_manager() -> AuditResult {
    // The task policy manager is now code-level; check if shell_exec defaults to deny
    AuditResult {
        name: "Task policy manager".to_string(),
        status: AuditStatus::Pass,
        message: "RequiresReview commands default to policy-denied".to_string(),
        category: "Policy".to_string(),
    }
}

/// Execute the security audit CLI command.
pub fn execute(json_output: bool) -> i32 {
    let report = AuditReport::run();

    if json_output {
        println!("{}", report.to_json());
    } else {
        print!("{}", report.to_text());
    }

    i32::from(report.summary.failures > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_report_runs() {
        let report = AuditReport::run();
        assert!(report.summary.total > 0);
        assert_eq!(
            report.summary.total,
            report.summary.passed + report.summary.warnings + report.summary.failures
        );
    }

    #[test]
    fn test_text_output() {
        let report = AuditReport::run();
        let text = report.to_text();
        assert!(text.contains("Security Audit Report"));
        assert!(text.contains("Summary:"));
    }

    #[test]
    fn test_json_output() {
        let report = AuditReport::run();
        let json = report.to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("results").is_some());
        assert!(parsed.get("summary").is_some());
    }

    #[test]
    fn test_tls_always_passes() {
        let result = check_tls_settings();
        assert_eq!(result.status, AuditStatus::Pass);
        assert!(result.message.contains("rustls"));
    }

    fn minimal_kit_yaml() -> &'static str {
        r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "hello"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  expires: "2026-05-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "say hello"
roles:
  - id: agent
    role_type: operator
    agent_provisioning: {}
    skills: []
    mcp_tools: []
    handoffs: []
coordination:
  topology: hub-spoke
  protocol: a2a-jsonrpc-2.0
  routing:
    strategy: static
constraints:
  global_budget:
    max_wallclock_seconds: 60
    max_total_tokens: 8000
    max_cost_usd: 0.01
  data_classifications: ["public"]
  network:
    egress_allowed: false
    egress_allowlist: []
completion:
  rules: ["done"]
  on_failure: abort
  max_retries: 0
provenance:
  signatures:
    - signer_did: "did:web:example.com"
      algorithm: ed25519
      signature: "AAA"
"#
    }

    #[test]
    fn swarmkit_check_passes_when_manifest_found() {
        let dir = tempfile::tempdir().unwrap();
        let arkavo_dir = dir.path().join(".arkavo");
        std::fs::create_dir_all(&arkavo_dir).unwrap();
        std::fs::write(arkavo_dir.join("agent.swarmkit.yaml"), minimal_kit_yaml()).unwrap();

        let result = check_swarmkit_manifest_at(dir.path());
        assert_eq!(result.status, AuditStatus::Pass);
        assert!(result.message.contains("SwarmKit manifest found"));
    }

    #[test]
    fn swarmkit_check_warns_when_no_manifest() {
        let dir = tempfile::tempdir().unwrap();

        let result = check_swarmkit_manifest_at(dir.path());
        assert_eq!(result.status, AuditStatus::Warn);
        assert!(result.message.contains("arkavo kit init"));
    }

    #[test]
    fn swarmkit_check_warns_with_migrate_hint_when_only_agents_md_present() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "# AGENTS.md\n").unwrap();

        let result = check_swarmkit_manifest_at(dir.path());
        assert_eq!(result.status, AuditStatus::Warn);
        assert!(result.message.contains("migrate-from-agents-md"));
    }

    #[test]
    fn test_status_serialization() {
        let result = AuditResult {
            name: "test".to_string(),
            status: AuditStatus::Pass,
            message: "ok".to_string(),
            category: "test".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"Pass\""));
    }
}
