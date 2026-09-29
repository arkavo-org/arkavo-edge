//! Security audit command for automated posture assessment
//!
//! Checks file permissions, the RPC endpoint (listen address, transport,
//! authentication, rate limiting), preflight moderation, memory encryption,
//! the kit and the shell command policy.
//! Outputs human-readable text or JSON for CI integration.
//!
//! A check passes only on something it observed. Where the control it is
//! named after does not exist, it says so.

use serde::Serialize;
use std::fmt::Write;
use std::path::{Path, PathBuf};

use crate::commands::agent::listen::BindAddress;

mod network;

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

fn result(name: &str, category: &str, status: AuditStatus, message: String) -> AuditResult {
    AuditResult {
        name: name.to_string(),
        status,
        message,
        category: category.to_string(),
    }
}

impl AuditReport {
    /// Run all security audit checks against the current directory, the one
    /// `arkavo agent` would discover its kit from. `bind` audits the agent
    /// as started with `--bind`.
    pub fn run(bind: Option<BindAddress>) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::run_at(&cwd, bind)
    }

    /// `cwd`-parameterized so tests can audit a directory of their own
    /// without depending on (or mutating) the process's working directory.
    fn run_at(cwd: &Path, bind: Option<BindAddress>) -> Self {
        let endpoint = network::effective_endpoint(cwd, bind);
        let results = vec![
            check_arkavo_dir_permissions(),
            network::check_bind(&endpoint),
            network::check_transport(&endpoint),
            network::check_rate_limiting(&crate::commands::agent::listen::rpc_rate_limit()),
            network::check_authentication(&endpoint),
            check_preflight_moderation(),
            check_memory_encryption(),
            check_swarmkit_manifest_at(cwd),
            check_api_keys_in_env(),
            check_shell_command_policy(),
            check_tool_isolation(),
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
    check_dir_permissions(&arkavo_dir())
}

fn check_dir_permissions(dir: &Path) -> AuditResult {
    let name = "Config directory";
    let category = "Filesystem";
    if !dir.exists() {
        return result(
            name,
            category,
            AuditStatus::Warn,
            "~/.arkavo/ does not exist".to_string(),
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match std::fs::metadata(dir) {
            Ok(meta) => {
                let mode = meta.mode() & 0o777;
                if mode == 0o700 {
                    result(
                        name,
                        category,
                        AuditStatus::Pass,
                        "~/.arkavo/ has correct permissions (0700)".to_string(),
                    )
                } else {
                    result(
                        name,
                        category,
                        AuditStatus::Fail,
                        format!("~/.arkavo/ has permissions {mode:o}, expected 0700"),
                    )
                }
            }
            Err(e) => result(
                name,
                category,
                AuditStatus::Warn,
                format!("~/.arkavo/ permissions could not be read: {e}"),
            ),
        }
    }

    #[cfg(not(unix))]
    {
        result(
            name,
            category,
            AuditStatus::Warn,
            "~/.arkavo/ exists; its access control is not inspected on this platform".to_string(),
        )
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

/// `cwd`-parameterized so tests can exercise every discovery outcome
/// without depending on (or mutating) the process's real working directory.
fn check_swarmkit_manifest_at(cwd: &Path) -> AuditResult {
    let name = "Agent config";
    let category = "Configuration";
    match arkavo_swarmkit::discover_kit_path(cwd) {
        // Finding the file is not enough: the agent refuses to start on a
        // manifest that does not load.
        Ok(path) => match arkavo_swarmkit::load_kit_file(&path) {
            Ok(_) => result(
                name,
                category,
                AuditStatus::Pass,
                format!("SwarmKit manifest found and valid: {}", path.display()),
            ),
            Err(err) => result(
                name,
                category,
                AuditStatus::Fail,
                format!("SwarmKit manifest does not load: {err}"),
            ),
        },
        Err(arkavo_swarmkit::DiscoverError::NotFound) => result(
            name,
            category,
            AuditStatus::Warn,
            "No SwarmKit manifest; agent runs with defaults — arkavo kit init <name>".to_string(),
        ),
        Err(err @ arkavo_swarmkit::DiscoverError::AgentsMdUnsupported { .. }) => {
            result(name, category, AuditStatus::Warn, err.to_string())
        }
        Err(err) => result(
            name,
            category,
            AuditStatus::Warn,
            format!("SwarmKit manifest discovery error: {err}"),
        ),
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

/// Classify probe commands with the shell tool itself. Nothing is executed:
/// classification is a pure function of the command text.
fn check_shell_command_policy() -> AuditResult {
    use arkavo_mcp_tools::shell_exec::{ApprovalResult, ShellExecTool};

    let name = "Shell command policy";
    let category = "Policy";
    let tool = ShellExecTool::new();

    let destructive = "rm -rf /";
    if !matches!(
        tool.classify_command(destructive),
        ApprovalResult::AutoBlocked(_)
    ) {
        return result(
            name,
            category,
            AuditStatus::Fail,
            format!("shell_exec does not block the destructive probe `{destructive}`"),
        );
    }

    let unrecognised = "make install";
    if tool.classify_command(unrecognised) != ApprovalResult::RequiresReview {
        return result(
            name,
            category,
            AuditStatus::Fail,
            format!("shell_exec does not send the unrecognised probe `{unrecognised}` to review"),
        );
    }

    result(
        name,
        category,
        AuditStatus::Pass,
        format!(
            "shell_exec blocks `{destructive}` and sends `{unrecognised}` to policy review \
             instead of approving it"
        ),
    )
}

/// A constant because tool dispatch (the conductor tool loop and the MCP
/// registry) never consults a sandbox: `ToolSandbox` has no caller and nothing
/// constructs a `TaskPolicyManager`, so there is no runtime state to inspect.
/// Warn rather than Fail keeps this file's convention that Fail marks a
/// misconfiguration the operator can correct.
fn check_tool_isolation() -> AuditResult {
    AuditResult {
        name: "Tool isolation".to_string(),
        status: AuditStatus::Warn,
        message: "No tool execution path is sandboxed; shell_exec and other process-spawning tools run as the agent's OS user with its files, network and environment (command allow/block lists are string heuristics, not confinement)".to_string(),
        category: "Tool Execution".to_string(),
    }
}

/// Execute the security audit CLI command. `bind` audits the agent as
/// started with `--bind`.
pub fn execute(json_output: bool, bind: Option<BindAddress>) -> i32 {
    let report = AuditReport::run(bind);

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
        let report = AuditReport::run(None);
        assert!(report.summary.total > 0);
        assert_eq!(
            report.summary.total,
            report.summary.passed + report.summary.warnings + report.summary.failures
        );
    }

    #[test]
    fn test_text_output() {
        let report = AuditReport::run(None);
        let text = report.to_text();
        assert!(text.contains("Security Audit Report"));
        assert!(text.contains("Summary:"));
    }

    #[test]
    fn test_json_output() {
        let report = AuditReport::run(None);
        let json = report.to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("results").is_some());
        assert!(parsed.get("summary").is_some());
    }

    /// Regression: the audit reported a hard-coded Pass named "Task policy
    /// manager" although no tool call is confined. Nothing constructs a
    /// `TaskPolicyManager` and `ToolSandbox` has no caller, so the report must
    /// say tool execution is unconfined.
    #[test]
    fn audit_reports_tool_execution_as_unconfined() {
        let report = AuditReport::run(None);
        let isolation = report
            .results
            .iter()
            .find(|r| r.name == "Tool isolation")
            .expect("audit must report tool isolation");
        assert_eq!(isolation.status, AuditStatus::Warn);
        assert!(
            isolation.message.contains("shell_exec"),
            "message must name the unconfined tool path: {}",
            isolation.message
        );
        assert!(
            report
                .results
                .iter()
                .all(|r| !(r.name == "Task policy manager" && r.status == AuditStatus::Pass)),
            "no Pass may be reported for an unwired policy manager"
        );
    }

    /// Regression: the report passed bind, TLS, rate limiting and policy
    /// checks that looked at nothing. A kit that exposes the endpoint must
    /// now fail the audit as a whole.
    #[test]
    fn a_kit_listening_on_the_network_fails_the_audit() {
        let dir = tempfile::tempdir().unwrap();
        let kit =
            minimal_kit_yaml().replacen("kit:", "runtime:\n  listen: \"0.0.0.0:8342\"\nkit:", 1);
        std::fs::write(dir.path().join("agent.swarmkit.yaml"), kit).unwrap();

        let report = AuditReport::run_at(dir.path(), None);

        assert!(report.summary.failures >= 3, "{}", report.to_text());
        let bind = report
            .results
            .iter()
            .find(|r| r.name == "Bind address")
            .expect("the bind check always runs");
        assert_eq!(bind.status, AuditStatus::Fail);
        assert!(bind.message.contains("0.0.0.0:8342"), "{}", bind.message);
    }

    fn check<'a>(report: &'a AuditReport, name: &str) -> &'a AuditResult {
        report
            .results
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("the {name} check always runs"))
    }

    /// The audit and startup must agree on the built-in loopback default.
    #[test]
    fn an_audit_with_no_kit_reports_the_loopback_default() {
        let dir = tempfile::tempdir().unwrap();
        let report = AuditReport::run_at(dir.path(), None);

        let bind = check(&report, "Bind address");
        assert_eq!(bind.status, AuditStatus::Pass, "{}", bind.message);
        assert!(bind.message.contains("127.0.0.1:0"), "{}", bind.message);
        assert_eq!(
            check(&report, "Transport encryption").status,
            AuditStatus::Pass
        );
        assert_eq!(check(&report, "Authentication").status, AuditStatus::Warn);
    }

    #[test]
    fn an_audit_of_a_loopback_bind_reports_loopback() {
        let dir = tempfile::tempdir().unwrap();
        let loopback = crate::commands::agent::listen::parse_bind("[::1]").unwrap();
        let report = AuditReport::run_at(dir.path(), Some(loopback));

        let bind = check(&report, "Bind address");
        assert_eq!(bind.status, AuditStatus::Pass, "{}", bind.message);
        assert!(bind.message.contains("[::1]:0"), "{}", bind.message);
        assert!(bind.message.contains("--bind"), "{}", bind.message);
        assert_eq!(
            check(&report, "Authentication").status,
            AuditStatus::Warn,
            "{}",
            report.to_text()
        );
    }

    #[test]
    fn no_check_claims_a_jwt_secret() {
        let dir = tempfile::tempdir().unwrap();
        let report = AuditReport::run_at(dir.path(), None);
        assert!(!report.to_text().contains("JWT_SECRET"));
    }

    #[test]
    fn checks_of_one_category_are_listed_together() {
        let dir = tempfile::tempdir().unwrap();
        let text = AuditReport::run_at(dir.path(), None).to_text();
        assert_eq!(text.matches("[Network]").count(), 1, "{text}");
    }

    #[test]
    fn shell_command_policy_passes_on_what_the_tool_classifies() {
        let check = check_shell_command_policy();
        assert_eq!(check.status, AuditStatus::Pass, "{}", check.message);
        assert!(check.message.contains("rm -rf /"));
    }

    #[cfg(unix)]
    #[test]
    fn config_directory_permissions_are_read_from_the_directory() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(check_dir_permissions(dir.path()).status, AuditStatus::Pass);

        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let open = check_dir_permissions(dir.path());
        assert_eq!(open.status, AuditStatus::Fail);
        assert!(open.message.contains("755"), "{}", open.message);

        let missing = check_dir_permissions(&dir.path().join("absent"));
        assert_eq!(missing.status, AuditStatus::Warn);
    }

    #[test]
    fn swarmkit_check_fails_when_the_manifest_does_not_load() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("agent.swarmkit.yaml"), "not: [a, kit").unwrap();

        let result = check_swarmkit_manifest_at(dir.path());
        assert_eq!(result.status, AuditStatus::Fail);
        assert!(
            result.message.contains("does not load"),
            "{}",
            result.message
        );
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
