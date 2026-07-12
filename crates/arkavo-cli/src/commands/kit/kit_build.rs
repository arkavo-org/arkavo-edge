//! `arkavo kit migrate-from-agents-md` — best-effort conversion of a legacy
//! AGENTS.md agent config into a SwarmKit manifest.
//!
//! Reuses `parse_legacy_agents_md` (`crate::commands::kit::legacy_agents_md`),
//! the CLI-local legacy markdown/YAML parser scheduled for deletion once this
//! migrate command is retired. This command is its last sanctioned caller —
//! do not add new callers.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use arkavo_protocol::agent_config::AgentMode;
use arkavo_swarmkit::{
    AgentProvisioning, AuthMode, KitRuntimeConfig, McpToolGrant, Model, Objective, RoleSpec,
    RuntimeMcpServer, RuntimeMode, validate,
};

use crate::commands::agent::{AgentConfig, McpServerConfig};
use crate::commands::kit::frontmatter::extract_runtime_extras;
use crate::commands::kit::legacy_agents_md::parse_legacy_agents_md;
use crate::commands::kit::model_map::hint_to_kit_model;

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

    let agents = parse_legacy_agents_md(&content)
        .map_err(|e| format!("failed to parse {}: {e}", in_path.display()))?;
    let Some(first) = agents.first() else {
        return Err(format!("no agent sections found in {}", in_path.display()).into());
    };

    // Frontmatter `preflight:`/`kas:`/`budget:` have no representation in
    // the line-based parser above (it treats them as unknown sections and
    // drops every line inside silently); recover them separately here
    // (finding 2) so they land in `runtime.*` instead of being lost.
    let extras = extract_runtime_extras(&content);

    let kit_name = first.name.clone();
    let goal = if first.purpose.trim().is_empty() {
        super::DEFAULT_GOAL.to_string()
    } else {
        first.purpose.clone()
    };

    let mut used_ids = HashSet::new();
    let mut mcp_servers: Vec<RuntimeMcpServer> = Vec::new();
    let mut mcp_conflicts: Vec<String> = Vec::new();
    let mut unmapped_models: Vec<(String, String)> = Vec::new();

    let roles: Vec<RoleSpec> = agents
        .iter()
        .map(|a| {
            let (role, unmapped_hint) = build_role(a, &mut used_ids);
            if let Some(hint) = unmapped_hint {
                unmapped_models.push((role.id.clone(), hint));
            }
            for server in &a.mcp_servers {
                let candidate = to_runtime_mcp_server(server);
                match mcp_servers.iter().find(|s| s.name == server.name) {
                    // Same name, same config: harmless duplicate, stay silent.
                    Some(existing) if *existing == candidate => {}
                    // Same name, different config: first wins, but the
                    // divergence must be reported (brief item 2).
                    Some(_) => mcp_conflicts.push(server.name.clone()),
                    None => mcp_servers.push(candidate),
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
            preflight: extras.preflight.clone(),
            kas: extras.kas.clone(),
            max_cost_per_session: extras.max_cost_per_session,
            max_cost_per_day: extras.max_cost_per_day,
            cloud_policy: extras.cloud_policy,
        }),
    );

    // Producer flow per manifest.rs: validate with an empty id, compute the
    // BLAKE3 id, then validate again to confirm the round-trip.
    validate(&manifest)?;
    manifest.compute_kit_id()?;
    validate(&manifest)?;

    let yaml = serde_yaml::to_string(&manifest)?;
    std::fs::write(out_path, yaml)?;

    let mut unmapped = unmapped_lines(&agents, &content, &unmapped_models, &mcp_conflicts);
    unmapped.extend(runtime_divergence_lines(&agents));
    unmapped.extend(extras.parse_errors);

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

/// Best-effort `model:` hint → kit `Model` mapping (brief item 4). Thin
/// wrapper over the shared `kit::model_map` table — the single source of
/// truth for this direction and its inverse (`model_map::kit_model_to_hint`,
/// used by the `arkavo agent -c <kit>` run path in `agent_kit.rs`) — so the
/// two CLI surfaces can never drift onto separate vocabularies.
fn map_model_hint(hint: &str) -> Option<Model> {
    hint_to_kit_model(hint)
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

/// A kit has exactly one `runtime` block, built from `agents[0]` only. If a
/// later AGENTS.md section declares a different `listen`, `mode`, or
/// `mdns`, that setting has nowhere to go and must be reported rather than
/// silently dropped (finding 1).
fn runtime_divergence_lines(agents: &[AgentConfig]) -> Vec<String> {
    let Some(first) = agents.first() else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for agent in agents.iter().skip(1) {
        if agent.listen != first.listen {
            lines.push(format!(
                "unmapped: listen (agent '{}' differs from kit-level runtime.listen)",
                agent.name
            ));
        }
        if agent.mode != first.mode {
            lines.push(format!(
                "unmapped: mode (agent '{}' differs from kit-level runtime.mode)",
                agent.name
            ));
        }
        if agent.mdns_enabled != first.mdns_enabled {
            lines.push(format!(
                "unmapped: mdns (agent '{}' differs from kit-level runtime.mdns)",
                agent.name
            ));
        }
    }
    lines
}

/// Reject a `kit init`/`agent init` name that could escape `.arkavo/` —
/// path separators or `..` turn the "kit slug" into a path (finding 3).
pub(super) fn validate_kit_name(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.contains(['/', '\\']) || trimmed.contains("..") {
        return Err(format!("invalid kit name {name:?}: no '/', '\\', '..', or empty").into());
    }
    Ok(())
}

/// Fields the legacy parser captures that have no SwarmKit home (brief item
/// 7). Reported once per field kind across the whole file, not once per
/// agent section, so a multi-agent migration doesn't spam duplicate lines.
fn unmapped_lines(
    agents: &[AgentConfig],
    raw_content: &str,
    unmapped_models: &[(String, String)],
    mcp_conflicts: &[String],
) -> Vec<String> {
    let mut lines = Vec::new();

    let mut conflict_names = mcp_conflicts.to_vec();
    conflict_names.sort_unstable();
    conflict_names.dedup();
    for name in &conflict_names {
        lines.push(format!(
            "unmapped: mcp_server '{name}' (conflicting definitions across agents; kept first)"
        ));
    }

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
    // content by parse_legacy_agents_md (every branch of that parser defaults
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
