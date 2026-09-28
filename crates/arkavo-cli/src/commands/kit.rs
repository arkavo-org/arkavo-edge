//! `arkavo kit init` / `arkavo kit validate` / `arkavo kit migrate-from-agents-md`
//! — author, check, and migrate-into SwarmKit manifests.
//!
//! This is the Phase S4 replacement for `arkavo agent init` (which is now
//! deprecated and delegates entirely to this module's `init_kit`, writing a
//! SwarmKit manifest rather than AGENTS.md — see
//! `commands::agent::deprecated_init`). The migration logic lives in
//! `kit_build`, and the manifest every producer starts from in
//! `manifest_template`.

use std::path::{Path, PathBuf};

use arkavo_swarmkit::{discover::ARKAVO_DIR, kit_id_for, load_kit_file, validate};

mod agents_md;
mod agents_md_markdown;
mod agents_md_yaml;
mod frontmatter;
mod kit_build;
mod manifest_template;
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
    let mut manifest = manifest_template::build_manifest(name);
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
