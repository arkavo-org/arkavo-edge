//! Exercises `arkavo_terminal::system_prompt_source`'s precedence: a
//! discovered SwarmKit kit's purpose text wins over `CLAUDE.md`, and
//! `CLAUDE.md` is only used as a fallback when no kit is present. Product
//! AGENTS.md is never read for this prompt (see `discover.rs`'s
//! `AgentsMdUnsupported`), so a lone AGENTS.md file behaves the same as no
//! config at all here.

use std::fs;
use tempfile::TempDir;

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
  goal: "You are a specialized AI agent for code analysis."
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
fn kit_purpose_takes_precedence_over_claude_md() {
    let temp_dir = TempDir::new().unwrap();
    fs::write(
        temp_dir.path().join("CLAUDE.md"),
        "# CLAUDE.md\n\nThis is a fallback prompt.",
    )
    .unwrap();

    let arkavo_dir = temp_dir.path().join(".arkavo");
    fs::create_dir_all(&arkavo_dir).unwrap();
    fs::write(arkavo_dir.join("agent.swarmkit.yaml"), minimal_kit_yaml()).unwrap();

    let prompt = arkavo_terminal::system_prompt_source(temp_dir.path())
        .expect("kit purpose should be found");
    assert!(prompt.contains("specialized AI agent for code analysis"));
    assert!(!prompt.contains("fallback prompt"));
}

#[test]
fn claude_md_is_fallback_when_no_kit_present() {
    let temp_dir = TempDir::new().unwrap();
    let claude_content = "# CLAUDE.md\n\nThis is a fallback prompt.";
    fs::write(temp_dir.path().join("CLAUDE.md"), claude_content).unwrap();

    let prompt = arkavo_terminal::system_prompt_source(temp_dir.path())
        .expect("CLAUDE.md should be used as fallback");
    assert_eq!(prompt, claude_content);
}

#[test]
fn agents_md_alone_does_not_satisfy_the_prompt_source() {
    // A lone AGENTS.md is unsupported agent config (see
    // `arkavo_swarmkit::DiscoverError::AgentsMdUnsupported`) — it must not
    // be read as the prompt source, only CLAUDE.md may serve as fallback.
    let temp_dir = TempDir::new().unwrap();
    fs::write(
        temp_dir.path().join("AGENTS.md"),
        "# AGENTS.md\n\nThis is a test agent prompt.",
    )
    .unwrap();

    assert!(arkavo_terminal::system_prompt_source(temp_dir.path()).is_none());
}

#[test]
fn no_kit_and_no_claude_md_returns_none() {
    let temp_dir = TempDir::new().unwrap();
    assert!(arkavo_terminal::system_prompt_source(temp_dir.path()).is_none());
}
