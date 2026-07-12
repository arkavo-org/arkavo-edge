//! `arkavo kit init` / `arkavo kit validate` / `arkavo kit migrate-from-agents-md`
//! — author, check, and migrate-into SwarmKit manifests.
//!
//! This is the Phase S4 replacement for `arkavo agent init` (which still
//! writes AGENTS.md; that command is deprecated separately). The migration
//! logic itself lives in the sibling `kit_build` module to keep this file
//! under the repo's per-file size limit.

use base64::Engine;
use rand::Rng;
use std::path::{Path, PathBuf};

use arkavo_swarmkit::coordination::{RoutingStrategy, Topology};
use arkavo_swarmkit::role::{Isolation, Sandbox};
use arkavo_swarmkit::{
    AgentProvisioning, Author, Budget, CompletionSpec, ConstraintsSpec, CoordinationSpec,
    GlobalBudget, KitMetadata, KitRuntimeConfig, Manifest, Model, NetworkConstraints, Objective,
    OnFailure, ProvenanceSpec, RoleSpec, Routing, Signature, Skill, SkillSource,
    discover::ARKAVO_DIR, kit_id_for, load_kit_file, validate,
};

mod kit_build;
mod legacy_agents_md;
mod legacy_agents_md_yaml;
mod model_map;
pub use kit_build::{MigrateReport, migrate_from_agents_md};
pub(crate) use model_map::kit_model_to_hint;

pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.is_empty() {
        print_usage();
        return Ok(());
    }
    match args[0].as_str() {
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(())
        }
        "init" => cmd_init(&args[1..]),
        "validate" => cmd_validate(&args[1..]),
        "migrate-from-agents-md" => cmd_migrate(&args[1..]),
        other => {
            eprintln!("Error: Unknown kit subcommand '{other}'");
            print_usage();
            Err(format!("Unknown kit subcommand: {other}").into())
        }
    }
}

fn cmd_init(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let Some(name) = args.first().filter(|a| !a.starts_with('-')) else {
        eprintln!("Error: kit name required");
        print_usage();
        return Err("Missing kit name".into());
    };

    let report = init_kit(Path::new("."), name)?;
    println!("Wrote {}", display_relative(&report.path));
    println!("kit.id: {}", report.kit_id);
    println!("Next: arkavo kit validate .arkavo/{name}.swarmkit.yaml");
    Ok(())
}

fn cmd_validate(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let Some(path) = args.first() else {
        eprintln!("Error: kit path required");
        print_usage();
        return Err("Missing kit path".into());
    };

    let report = validate_kit(Path::new(path))?;
    println!("kit: {}", report.kit_name);
    println!("kit.id: {}", report.kit_id);
    println!("kit.id matches recomputed hash: {}", report.id_matches);
    Ok(())
}

/// `--in <path> --out <path>` are the only accepted flags (contractually
/// pinned: `discover.rs`'s `AgentsMdUnsupported` error message tells users
/// to run exactly this invocation shape).
fn cmd_migrate(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut in_path: Option<PathBuf> = None;
    let mut out_path: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--in" => {
                i += 1;
                in_path = args.get(i).map(PathBuf::from);
            }
            "--out" => {
                i += 1;
                out_path = args.get(i).map(PathBuf::from);
            }
            other => {
                eprintln!("Error: Unknown argument '{other}'");
                print_usage();
                return Err(format!("Unknown argument: {other}").into());
            }
        }
        i += 1;
    }

    let (Some(in_path), Some(out_path)) = (in_path, out_path) else {
        eprintln!("Error: --in <path> and --out <path> are both required");
        print_usage();
        return Err("Missing --in/--out".into());
    };

    let report = migrate_from_agents_md(&in_path, &out_path)?;
    println!("Wrote {}", display_relative(&report.path));
    println!("kit.id: {}", report.kit_id);

    if report.unmapped.is_empty() {
        return Ok(());
    }
    for line in &report.unmapped {
        eprintln!("{line}");
    }
    Err(format!(
        "{} field(s) from {} could not be migrated; see stderr for details",
        report.unmapped.len(),
        in_path.display()
    )
    .into())
}

/// Strip a leading `./` so CLI output reads as a normal relative path
/// (`base_dir` is `.` when invoked from the CLI, an absolute tempdir in tests).
fn display_relative(p: &Path) -> String {
    p.strip_prefix(".").unwrap_or(p).display().to_string()
}

fn print_usage() {
    println!("Arkavo Kit - Author, validate, and migrate SwarmKit manifests");
    println!();
    println!("USAGE:");
    println!("    arkavo kit init <name>");
    println!("    arkavo kit validate <path>");
    println!("    arkavo kit migrate-from-agents-md --in <path> --out <path>");
    println!();
    println!("SUBCOMMANDS:");
    println!(
        "    init <name>                            Write a minimal single-role kit to .arkavo/<name>.swarmkit.yaml"
    );
    println!("    validate <path>                         Load and validate a kit file");
    println!(
        "    migrate-from-agents-md --in <in> --out <out>  Best-effort convert an AGENTS.md file into a kit"
    );
    println!("    help                                     Print this help message");
}

/// Result of a successful `kit init`.
pub struct KitInitReport {
    pub path: PathBuf,
    pub kit_id: String,
}

/// Write a minimal single-role SwarmKit manifest to
/// `<base_dir>/.arkavo/<name>.swarmkit.yaml`. Fails without touching the
/// filesystem further if the target file already exists.
pub fn init_kit(base_dir: &Path, name: &str) -> Result<KitInitReport, Box<dyn std::error::Error>> {
    kit_build::validate_kit_name(name)?;
    let arkavo_dir = base_dir.join(ARKAVO_DIR);
    std::fs::create_dir_all(&arkavo_dir)?;

    let target = arkavo_dir.join(format!("{name}.swarmkit.yaml"));
    if target.exists() {
        return Err(format!(
            "{} already exists; refusing to overwrite",
            display_relative(&target)
        )
        .into());
    }

    // Producer flow per manifest.rs: author with kit.id empty, validate,
    // compute the BLAKE3 id, then validate again to confirm the round-trip.
    let mut manifest = build_manifest(name);
    validate(&manifest)?;
    manifest.compute_kit_id()?;
    validate(&manifest)?;

    let yaml = serde_yaml::to_string(&manifest)?;
    std::fs::write(&target, yaml)?;

    Ok(KitInitReport {
        path: target,
        kit_id: manifest.kit.id,
    })
}

/// Result of a successful `kit validate`.
pub struct KitValidateReport {
    pub kit_name: String,
    pub kit_id: String,
    pub id_matches: bool,
}

/// Load and validate a kit file, then confirm the declared `kit.id` matches the recomputed hash.
///
/// `load_kit_file` already enforces this internally whenever `kit.id` is
/// non-empty, so a mismatch (or invalid YAML / cross-block validation
/// failure) surfaces as an `Err` from that call; the explicit recompute
/// below only matters for the edge case of an unassigned (empty) `kit.id`.
pub fn validate_kit(path: &Path) -> Result<KitValidateReport, Box<dyn std::error::Error>> {
    let manifest = load_kit_file(path)?.manifest;
    let expected = kit_id_for(&manifest)?;
    let id_matches = expected == manifest.kit.id;

    if !id_matches {
        return Err(format!(
            "kit.id mismatch: declared {:?}, recomputed {:?}",
            manifest.kit.id, expected
        )
        .into());
    }

    Ok(KitValidateReport {
        kit_name: manifest.kit.name,
        kit_id: manifest.kit.id,
        id_matches,
    })
}

/// Default single-role goal text; shared fallback `objective.goal` for `kit_build`.
const DEFAULT_GOAL: &str = "Introduce yourself and assist with the tasks the user brings to you";
/// Default identity-skill instructions; shared fallback per-role instructions for `kit_build`.
const DEFAULT_IDENTITY_INSTRUCTIONS: &str =
    "You are a helpful agent. Introduce yourself and assist with the tasks the user brings to you.";

fn build_manifest(name: &str) -> Manifest {
    manifest_skeleton(
        name,
        format!("Locally authored single-role agent kit for {name}"),
        Objective {
            goal: DEFAULT_GOAL.to_string(),
            success_criteria: vec![
                "responds helpfully and stays within its configured budget".to_string(),
            ],
        },
        vec![primary_role()],
        Some(KitRuntimeConfig {
            local_dev: Some(true),
            mdns: Some(true),
            ..Default::default()
        }),
    )
}

/// The kit-metadata / coordination / constraints / completion / provenance
/// skeleton shared by every producer path (`kit init`, `kit
/// migrate-from-agents-md`). Producers own `objective` and `roles`; `runtime`
/// is optional since not every caller sets it.
fn manifest_skeleton(
    name: &str,
    description: String,
    objective: Objective,
    roles: Vec<RoleSpec>,
    runtime: Option<KitRuntimeConfig>,
) -> Manifest {
    let now = chrono::Utc::now();
    let created = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let expires =
        (now + chrono::Duration::days(90)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let author_did = "did:web:local.arkavo.dev".to_string();

    Manifest {
        spec_version: "1.0.0".to_string(),
        kit: KitMetadata {
            id: String::new(),
            name: name.to_string(),
            version: "0.1.0".to_string(),
            description: Some(description),
            authors: vec![Author {
                did: author_did.clone(),
                name: Some("Local Author".to_string()),
            }],
            created,
            expires: Some(expires),
            nonce: generate_nonce(),
        },
        objective,
        inputs: vec![],
        deliverables: vec![],
        roles,
        coordination: CoordinationSpec {
            topology: Topology::HubSpoke,
            protocol: "a2a-jsonrpc-2.0".to_string(),
            routing: Routing {
                strategy: RoutingStrategy::Static,
                parameters: None,
            },
            context_sharing: None,
        },
        constraints: ConstraintsSpec {
            global_budget: GlobalBudget {
                max_wallclock_seconds: 300,
                max_total_tokens: 100_000,
                max_cost_usd: 0.50,
            },
            data_classifications: vec!["public".to_string()],
            jurisdiction: vec![],
            network: NetworkConstraints {
                egress_allowed: false,
                egress_allowlist: vec![],
            },
        },
        pricing: vec![],
        evaluation: None,
        proposal_governance: None,
        runtime,
        completion: CompletionSpec {
            rules: vec!["objective addressed".to_string()],
            on_failure: OnFailure::Abort,
            max_retries: 0,
        },
        provenance: ProvenanceSpec {
            c2pa_assertions: vec![],
            signatures: vec![Signature {
                signer_did: author_did,
                algorithm: "ed25519".to_string(),
                signature: "AAA".to_string(),
            }],
        },
    }
}

fn primary_role() -> RoleSpec {
    RoleSpec {
        id: "agent".to_string(),
        role_type: "operator".to_string(),
        plane: None,
        description: Some("Primary agent".to_string()),
        agent_provisioning: AgentProvisioning {
            model: Some(default_model()),
            inference: None,
            budget: Some(default_budget()),
            tool_use: None,
            context: None,
            observability: None,
            isolation: Some(default_isolation()),
            failure: None,
        },
        skills: vec![identity_skill(DEFAULT_IDENTITY_INSTRUCTIONS)],
        mcp_tools: vec![],
        tdf_attribute_release_policy: None,
        handoffs: vec![],
        context_scope: None,
    }
}

/// Default local edge model. Also the migrate-from-agents-md fallback for
/// unmapped or absent `model:` hints (brief item 4).
fn default_model() -> Model {
    Model {
        family: "ministral".to_string(),
        size: Some("3B".to_string()),
        quantization: None,
        backend: Some("llama.cpp".to_string()),
        fallback: None,
    }
}

fn default_budget() -> Budget {
    Budget {
        max_inference_calls: Some(32),
        max_wallclock_ms: None,
        max_total_tokens: Some(100_000),
    }
}

fn default_isolation() -> Isolation {
    Isolation {
        sandbox: Some(Sandbox::Process),
        fs_writable: vec![],
        network_egress: Some(false),
    }
}

fn identity_skill(instructions: &str) -> Skill {
    Skill {
        id: "skill:identity".to_string(),
        version: "0.1.0".to_string(),
        source: SkillSource::Inline,
        payload: Some(serde_json::json!({
            "name": "identity",
            "description": "System identity",
            "instructions": instructions,
            "resources": [],
        })),
        signature: None,
        signed_by: None,
    }
}

/// Replay-prevention nonce (spec §4.1): 16 random bytes, base64url no-pad.
fn generate_nonce() -> String {
    let bytes: [u8; 16] = rand::thread_rng().r#gen();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_manifest_validates_with_empty_id() {
        let manifest = build_manifest("unit-test-agent");
        validate(&manifest).expect("freshly built manifest should validate before id assignment");
    }

    #[test]
    fn generate_nonce_is_nonempty_and_varies() {
        let a = generate_nonce();
        let b = generate_nonce();
        assert!(!a.is_empty());
        assert_ne!(a, b, "nonces should be randomly generated");
    }
}
