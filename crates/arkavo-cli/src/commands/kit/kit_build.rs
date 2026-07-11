//! `arkavo kit migrate-from-agents-md` — best-effort conversion of a legacy
//! AGENTS.md agent config into a SwarmKit manifest.
//!
//! Reuses `parse_agents_config` (`crate::commands::agent`), the CLI-local
//! legacy markdown/YAML parser scheduled for deletion once every product
//! AGENTS.md caller has migrated onto SwarmKit. This command is its last
//! sanctioned caller — do not add new callers.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use arkavo_protocol::agent_config::AgentMode;
use arkavo_router::ModelChoice;
use arkavo_swarmkit::{
    AgentProvisioning, AuthMode, KitRuntimeConfig, McpToolGrant, Model, Objective, RoleSpec,
    RuntimeMcpServer, RuntimeMode, validate,
};

use crate::commands::agent::{self, AgentConfig, McpServerConfig};

/// Result of a `migrate-from-agents-md` run.
///
/// `unmapped` is non-empty when some legacy field had no SwarmKit
/// representation; the kit is still written in that case (best-effort) —
/// the CLI layer turns a non-empty list into a non-zero process exit so
/// scripts notice (brief item 7).
pub struct MigrateReport {
    pub path: PathBuf,
    pub kit_id: String,
    pub unmapped: Vec<String>,
}

/// Convert the AGENTS.md file at `in_path` into a SwarmKit manifest written
/// to `out_path`.
///
/// Refuses to overwrite an existing `out_path` and refuses to write into a
/// missing parent directory (no implicit `mkdir -p`, per brief item 9).
pub fn migrate_from_agents_md(
    in_path: &Path,
    out_path: &Path,
) -> Result<MigrateReport, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(in_path)
        .map_err(|e| format!("failed to read {}: {e}", in_path.display()))?;

    if out_path.exists() {
        return Err(format!(
            "{} already exists; refusing to overwrite",
            out_path.display()
        )
        .into());
    }
    if let Some(parent) = out_path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        return Err(format!("parent directory {} does not exist", parent.display()).into());
    }

    let agents = agent::parse_agents_config(&content)
        .map_err(|e| format!("failed to parse {}: {e}", in_path.display()))?;
    let Some(first) = agents.first() else {
        return Err(format!("no agent sections found in {}", in_path.display()).into());
    };

    let kit_name = first.name.clone();
    let goal = if first.purpose.trim().is_empty() {
        super::DEFAULT_GOAL.to_string()
    } else {
        first.purpose.clone()
    };

    let mut used_ids = HashSet::new();
    let mut mcp_servers: Vec<RuntimeMcpServer> = Vec::new();
    let mut seen_server_names = HashSet::new();
    let mut unmapped_models: Vec<(String, String)> = Vec::new();

    let roles: Vec<RoleSpec> = agents
        .iter()
        .map(|a| {
            let (role, unmapped_hint) = build_role(a, &mut used_ids);
            if let Some(hint) = unmapped_hint {
                unmapped_models.push((role.id.clone(), hint));
            }
            for server in &a.mcp_servers {
                if seen_server_names.insert(server.name.clone()) {
                    mcp_servers.push(to_runtime_mcp_server(server));
                }
            }
            role
        })
        .collect();

    let mut manifest = super::manifest_skeleton(
        &kit_name,
        format!(
            "Migrated from {} by arkavo kit migrate-from-agents-md",
            in_path.display()
        ),
        Objective {
            goal,
            success_criteria: vec![
                "responds helpfully and stays within its configured budget".to_string(),
            ],
        },
        roles,
        Some(KitRuntimeConfig {
            local_dev: Some(true),
            mode: Some(to_runtime_mode(&first.mode)),
            listen: Some(first.listen.clone()),
            mdns: Some(first.mdns_enabled),
            mcp_servers,
            ..Default::default()
        }),
    );

    // Producer flow per manifest.rs: validate with an empty id, compute the
    // BLAKE3 id, then validate again to confirm the round-trip.
    validate(&manifest)?;
    manifest.compute_kit_id()?;
    validate(&manifest)?;

    let yaml = serde_yaml::to_string(&manifest)?;
    std::fs::write(out_path, yaml)?;

    let unmapped = unmapped_lines(&agents, &content, &unmapped_models);

    Ok(MigrateReport {
        path: out_path.to_path_buf(),
        kit_id: manifest.kit.id,
        unmapped,
    })
}

/// Build one role from one AGENTS.md agent section. Returns the unmapped
/// model hint (if any) alongside the role so the caller can attribute it.
fn build_role(agent: &AgentConfig, used_ids: &mut HashSet<String>) -> (RoleSpec, Option<String>) {
    let id = unique_slug(&agent.name, used_ids);

    let instructions = if agent.purpose.trim().is_empty() {
        super::DEFAULT_IDENTITY_INSTRUCTIONS.to_string()
    } else {
        agent.purpose.clone()
    };

    let (model, unmapped_hint) = if agent.model.trim().is_empty() {
        (super::default_model(), None)
    } else {
        match map_model_hint(&agent.model) {
            Some(model) => (model, None),
            None => (super::default_model(), Some(agent.model.clone())),
        }
    };

    let mcp_tools = agent
        .mcp_servers
        .iter()
        .map(|s| McpToolGrant {
            server: s.name.clone(),
            // We only know the server's identity from AGENTS.md, never its
            // actual tool names — an empty allowlist is the "no fabrication"
            // choice (see McpToolGrant docs: unlisted tools are a no-op, not
            // an error, so this is safely tightenable by hand later).
            tools: vec![],
            auth: AuthMode::Delegated,
        })
        .collect();

    let role = RoleSpec {
        id,
        role_type: "operator".to_string(),
        plane: None,
        description: Some(format!("Migrated from AGENTS.md agent {:?}", agent.name)),
        agent_provisioning: AgentProvisioning {
            model: Some(model),
            inference: None,
            budget: Some(super::default_budget()),
            tool_use: None,
            context: None,
            observability: None,
            isolation: Some(super::default_isolation()),
            failure: None,
        },
        skills: vec![super::identity_skill(&instructions)],
        mcp_tools,
        tdf_attribute_release_policy: None,
        handoffs: vec![],
        context_scope: None,
    };

    (role, unmapped_hint)
}

/// Slugify `name` into a role id, then disambiguate against ids already
/// used by earlier roles in the same kit (`-2`, `-3`, ...).
fn unique_slug(name: &str, used: &mut HashSet<String>) -> String {
    let base = slugify(name);
    let mut candidate = base.clone();
    let mut n = 2;
    while used.contains(&candidate) {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    used.insert(candidate.clone());
    candidate
}

fn slugify(input: &str) -> String {
    let mut out = String::new();
    for c in input.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_end_matches('-');
    if trimmed.is_empty() {
        "agent".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Best-effort `model:` hint → kit `Model` mapping (brief item 4). Reuses
/// `arkavo_router::ModelChoice::from_name` to recognize known aliases, but
/// re-derives family/size in the kit's own vocabulary (e.g. `"ministral"`,
/// not the router's generic vendor family `"mistral"`) rather than calling
/// `ModelChoice::family()`, which serves a different purpose (routing/prompt
/// lookups) and would produce the wrong string here.
///
/// Only covers the locally-hosted edge models this CLI actually provisions
/// (see CLAUDE.md's "Local Edge Models" section); cloud model hints (Claude,
/// Gemini, Grok, ...) are intentionally left unmapped since `agent_provisioning.model`
/// describes a local backend, not a cloud API id.
fn map_model_hint(hint: &str) -> Option<Model> {
    let (family, size) = match ModelChoice::from_name(hint)? {
        ModelChoice::LocalMinistral3B => ("ministral", "3B"),
        ModelChoice::LocalMinistral8B => ("ministral", "8B"),
        ModelChoice::LocalGemma4E2B => ("gemma", "E2B"),
        ModelChoice::LocalGemma4E4B => ("gemma", "E4B"),
        ModelChoice::LocalGemma4_12B => ("gemma", "12B"),
        ModelChoice::LocalQwen3 => ("qwen", "0.8B"),
        ModelChoice::LocalQwen35_9B => ("qwen", "9B"),
        ModelChoice::LocalQwen35_27B => ("qwen", "27B"),
        _ => return None,
    };
    Some(Model {
        family: family.to_string(),
        size: Some(size.to_string()),
        quantization: None,
        backend: Some("llama.cpp".to_string()),
        fallback: None,
    })
}

fn to_runtime_mode(mode: &AgentMode) -> RuntimeMode {
    match mode {
        AgentMode::Specialist => RuntimeMode::Specialist,
        AgentMode::Orchestrator => RuntimeMode::Orchestrator,
    }
}

fn to_runtime_mcp_server(s: &McpServerConfig) -> RuntimeMcpServer {
    RuntimeMcpServer {
        name: s.name.clone(),
        command: s.command.clone(),
        args: s.args.clone(),
        url: s.url.clone(),
    }
}

/// Fields the legacy parser captures that have no SwarmKit home (brief item
/// 7). Reported once per field kind across the whole file, not once per
/// agent section, so a multi-agent migration doesn't spam duplicate lines.
fn unmapped_lines(
    agents: &[AgentConfig],
    raw_content: &str,
    unmapped_models: &[(String, String)],
) -> Vec<String> {
    let mut lines = Vec::new();

    let mut key_names: Vec<&str> = agents
        .iter()
        .flat_map(|a| a.api_keys.keys())
        .map(String::as_str)
        .collect();
    key_names.sort_unstable();
    key_names.dedup();
    if !key_names.is_empty() {
        lines.push(format!(
            "unmapped: api_keys (API keys are env-only; set {} etc.)",
            key_names.join(", ")
        ));
    }

    if agents.iter().any(|a| a.swarm.is_some()) {
        lines.push(
            "unmapped: swarm (no kit field for the legacy learning-isolation domain identifier)"
                .to_string(),
        );
    }
    if agents.iter().any(|a| !a.peers.is_empty()) {
        lines.push(
            "unmapped: peers (static peer list has no kit home; discovery is mDNS/A2A-driven)"
                .to_string(),
        );
    }
    if agents.iter().any(|a| !a.a2a_enabled) {
        lines.push("unmapped: a2a_enabled (no kit field toggles A2A serving)".to_string());
    }
    if agents.iter().any(|a| a.a2a_service_type.is_some()) {
        lines.push(
            "unmapped: a2a_service_type (no kit field for a custom mDNS service type)".to_string(),
        );
    }
    // `quiet` is a CLI runtime flag, never actually parsed from AGENTS.md
    // content by parse_agents_config (every branch of that parser defaults
    // it to `true` regardless of file content), so a parsed AgentConfig can
    // never distinguish "explicitly set" from "default" for this field. A
    // raw content scan is the only way to detect an explicit `quiet:` key,
    // per the brief's escape hatch for fields the parser output can't
    // disambiguate.
    if raw_content.lines().any(|l| l.trim().starts_with("quiet:")) {
        lines.push("unmapped: quiet (verbosity is a CLI flag, not kit-representable)".to_string());
    }

    for (role_id, hint) in unmapped_models {
        lines.push(format!(
            "unmapped: model (role {role_id:?}: {hint:?} not recognized; falling back to ministral/3B)"
        ));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_lowercases_and_collapses_separators() {
        assert_eq!(slugify("Alert Manager!"), "alert-manager");
        assert_eq!(slugify("already-slug"), "already-slug");
        assert_eq!(slugify("---"), "agent");
        assert_eq!(slugify(""), "agent");
    }

    #[test]
    fn unique_slug_disambiguates_collisions() {
        let mut used = HashSet::new();
        assert_eq!(unique_slug("worker", &mut used), "worker");
        assert_eq!(unique_slug("worker", &mut used), "worker-2");
        assert_eq!(unique_slug("worker", &mut used), "worker-3");
    }

    #[test]
    fn map_model_hint_recognizes_known_local_models_only() {
        let model = map_model_hint("ministral-3b").expect("known hint should map");
        assert_eq!(model.family, "ministral");
        assert_eq!(model.size.as_deref(), Some("3B"));

        assert!(
            map_model_hint("claude-sonnet-4-5-20250929").is_none(),
            "cloud model hints must stay unmapped"
        );
        assert!(
            map_model_hint("totally-unknown-model").is_none(),
            "unrecognized hints must stay unmapped"
        );
    }
}
