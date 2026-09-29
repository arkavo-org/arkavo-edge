//! `arkavo kit init` / `arkavo kit validate` / `arkavo kit migrate-from-agents-md`
//! — author, check, and migrate-into SwarmKit manifests.
//!
//! This is the Phase S4 replacement for `arkavo agent init` (which is now
//! deprecated and delegates entirely to this module's `init_kit`, writing a
//! SwarmKit manifest rather than AGENTS.md — see
//! `commands::agent::deprecated_init`). The migration logic lives in
//! `kit_build`, and the manifest every producer starts from in
//! `manifest_template`.

use std::io::Write;
use std::path::{Path, PathBuf};

use arkavo_swarmkit::{discover::ARKAVO_DIR, validate};

mod agents_md;
mod agents_md_markdown;
mod agents_md_yaml;
mod frontmatter;
mod kit_build;
mod manifest_template;
mod model_map;
mod usage;
mod validate_cmd;
pub use kit_build::{MigrateReport, migrate_from_agents_md};
pub(crate) use model_map::kit_model_to_hint;
pub use validate_cmd::{KitValidateReport, validate_kit, validate_kit_at};

/// Words that are subcommands or help requests. `kit init help` would
/// otherwise write a kit called "help" when the user was asking for help.
const RESERVED_NAMES: &[&str] = &["help", "init", "validate", "migrate-from-agents-md"];

/// A failure is reported only through the returned error: the caller prints
/// it, so printing it here as well would show the user one mistake twice.
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    run(Path::new("."), args, &mut std::io::stdout().lock())
}

/// [`execute`] with the directory `kit init` writes under and the output
/// stream supplied, so the command line can be exercised without touching
/// the working directory.
fn run(
    base_dir: &Path,
    args: &[String],
    out: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some((subcommand, rest)) = args.split_first() else {
        out.write_all(usage::KIT.as_bytes())?;
        return Ok(());
    };
    let (help, command): (&str, Command) = match subcommand.as_str() {
        "-h" | "--help" | "help" => {
            out.write_all(usage::KIT.as_bytes())?;
            return Ok(());
        }
        "init" => (usage::INIT, cmd_init),
        "validate" => (usage::VALIDATE, cmd_validate),
        "migrate-from-agents-md" => (usage::MIGRATE, cmd_migrate),
        other => {
            return Err(usage_error(
                &format!("unknown kit subcommand '{other}'"),
                "arkavo kit --help",
            ));
        }
    };
    if rest.iter().any(|arg| arg == "-h" || arg == "--help") {
        out.write_all(help.as_bytes())?;
        return Ok(());
    }
    command(base_dir, rest, out)
}

type Command = fn(&Path, &[String], &mut dyn Write) -> Result<(), Box<dyn std::error::Error>>;

fn usage_error(message: &str, help_command: &str) -> Box<dyn std::error::Error> {
    format!("{message}; run '{help_command}' for usage").into()
}

fn reject_unknown_options(
    args: &[String],
    help_command: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.iter().find(|arg| arg.starts_with('-')) {
        Some(option) => Err(usage_error(
            &format!("unknown option '{option}'"),
            help_command,
        )),
        None => Ok(()),
    }
}

fn cmd_init(
    base_dir: &Path,
    args: &[String],
    out: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    const HELP: &str = "arkavo kit init --help";
    reject_unknown_options(args, HELP)?;
    let name = match args {
        [name] => name,
        [] => return Err(usage_error("kit name required", HELP)),
        [_, extra, ..] => {
            return Err(usage_error(
                &format!("unexpected argument '{extra}'; kit init takes one name"),
                HELP,
            ));
        }
    };

    let report = init_kit(base_dir, name)?;
    let path = display_relative(&report.path);
    writeln!(out, "Wrote {path}")?;
    writeln!(out, "kit.id: {}", report.kit_id)?;
    writeln!(out, "Next: arkavo kit validate {path}")?;
    writeln!(out, "Run:  arkavo agent -c {path}")?;
    Ok(())
}

/// Validates every path before reporting failure, so one bad file does not
/// hide the state of the ones after it.
fn cmd_validate(
    _base_dir: &Path,
    args: &[String],
    out: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    const HELP: &str = "arkavo kit validate --help";
    reject_unknown_options(args, HELP)?;
    if args.is_empty() {
        return Err(usage_error("kit path required", HELP));
    }

    let mut failures = Vec::new();
    for (index, path) in args.iter().enumerate() {
        match validate_kit(Path::new(path)) {
            Ok(report) => {
                if args.len() > 1 {
                    if index > 0 {
                        writeln!(out)?;
                    }
                    writeln!(out, "{path}")?;
                }
                for line in report.lines() {
                    writeln!(out, "{line}")?;
                }
            }
            Err(err) => failures.push(err.to_string()),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n").into())
    }
}

/// `--in <path> --out <path>` are the only accepted flags (contractually
/// pinned: `discover.rs`'s `AgentsMdUnsupported` error message tells users
/// to run exactly this invocation shape).
fn cmd_migrate(
    _base_dir: &Path,
    args: &[String],
    out: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    const HELP: &str = "arkavo kit migrate-from-agents-md --help";
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
                return Err(usage_error(&format!("unknown argument '{other}'"), HELP));
            }
        }
        i += 1;
    }

    let (Some(in_path), Some(out_path)) = (in_path, out_path) else {
        return Err(usage_error(
            "--in <path> and --out <path> are both required",
            HELP,
        ));
    };

    let report = migrate_from_agents_md(&in_path, &out_path)?;
    writeln!(out, "Wrote {}", display_relative(&report.path))?;
    writeln!(out, "kit.id: {}", report.kit_id)?;

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
    if RESERVED_NAMES
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(name.trim()))
    {
        return Err(format!(
            "invalid kit name {name:?}: reserved word; run 'arkavo kit init --help' for usage"
        )
        .into());
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    struct Outcome {
        stdout: String,
        result: Result<(), String>,
    }

    fn run_in(base_dir: &Path, args: &[&str]) -> Outcome {
        let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        let mut out = Vec::new();
        let result = run(base_dir, &args, &mut out).map_err(|e| e.to_string());
        Outcome {
            stdout: String::from_utf8(out).unwrap(),
            result,
        }
    }

    fn kit_files(dir: &Path) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(dir.join(ARKAVO_DIR)) else {
            return Vec::new();
        };
        entries
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    /// Regression: `kit validate --help` tried to read a file called
    /// `--help`, `kit init --help` exited 1, and `kit migrate-from-agents-md
    /// --help` reported an unknown argument.
    #[test]
    fn every_subcommand_prints_its_own_help_and_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        for (subcommand, expected) in [
            ("init", usage::INIT),
            ("validate", usage::VALIDATE),
            ("migrate-from-agents-md", usage::MIGRATE),
        ] {
            for flag in ["--help", "-h"] {
                let outcome = run_in(dir.path(), &[subcommand, flag]);
                assert_eq!(outcome.result, Ok(()), "kit {subcommand} {flag}");
                assert_eq!(outcome.stdout, expected, "kit {subcommand} {flag}");
            }
        }
        assert!(kit_files(dir.path()).is_empty(), "help must write nothing");
    }

    #[test]
    fn help_flag_wins_wherever_it_appears() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = run_in(dir.path(), &["init", "demo", "--help"]);
        assert_eq!(outcome.result, Ok(()));
        assert_eq!(outcome.stdout, usage::INIT);
        assert!(kit_files(dir.path()).is_empty());
    }

    #[test]
    fn top_level_help_forms_print_the_kit_usage() {
        let dir = tempfile::tempdir().unwrap();
        for args in [&[][..], &["help"], &["--help"], &["-h"]] {
            let outcome = run_in(dir.path(), args);
            assert_eq!(outcome.result, Ok(()));
            assert_eq!(outcome.stdout, usage::KIT);
        }
    }

    /// Regression: `kit init help` wrote `.arkavo/help.swarmkit.yaml`.
    #[test]
    fn init_rejects_reserved_words_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["help", "HELP", "init", "validate", "migrate-from-agents-md"] {
            let outcome = run_in(dir.path(), &["init", name]);
            let err = outcome.result.unwrap_err();
            assert!(err.contains("reserved word"), "{name}: {err}");
            assert_eq!(outcome.stdout, "", "{name}");
        }
        assert!(kit_files(dir.path()).is_empty());
        assert!(init_kit(dir.path(), "help").is_err());
    }

    #[test]
    fn init_accepts_a_name_that_only_contains_a_reserved_word() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = run_in(dir.path(), &["init", "help-desk"]);
        assert_eq!(outcome.result, Ok(()));
        assert_eq!(kit_files(dir.path()), ["help-desk.swarmkit.yaml"]);
    }

    /// Regression: a usage mistake printed an error, the whole usage text
    /// and then a second, differently worded error.
    #[test]
    fn usage_mistakes_return_one_error_line_and_print_nothing() {
        let dir = tempfile::tempdir().unwrap();
        for (args, expected) in [
            (
                &["init"][..],
                "kit name required; run 'arkavo kit init --help' for usage",
            ),
            (
                &["init", "--force"],
                "unknown option '--force'; run 'arkavo kit init --help' for usage",
            ),
            (
                &["init", "one", "two"],
                "unexpected argument 'two'; kit init takes one name; \
                 run 'arkavo kit init --help' for usage",
            ),
            (
                &["validate"],
                "kit path required; run 'arkavo kit validate --help' for usage",
            ),
            (
                &["migrate-from-agents-md", "AGENTS.md"],
                "unknown argument 'AGENTS.md'; \
                 run 'arkavo kit migrate-from-agents-md --help' for usage",
            ),
            (
                &["migrate-from-agents-md", "--in", "AGENTS.md"],
                "--in <path> and --out <path> are both required; \
                 run 'arkavo kit migrate-from-agents-md --help' for usage",
            ),
            (
                &["frobnicate"],
                "unknown kit subcommand 'frobnicate'; run 'arkavo kit --help' for usage",
            ),
        ] {
            let outcome = run_in(dir.path(), args);
            assert_eq!(outcome.result, Err(expected.to_string()), "{args:?}");
            assert_eq!(outcome.stdout, "", "{args:?} must not print usage");
        }
        assert!(kit_files(dir.path()).is_empty());
    }

    /// Regression: `kit init` ended with the validate hint and never said
    /// how to start the kit.
    #[test]
    fn init_output_ends_with_the_command_that_runs_the_kit() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = run_in(dir.path(), &["init", "demo"]);
        assert_eq!(outcome.result, Ok(()));

        let path = dir.path().join(ARKAVO_DIR).join("demo.swarmkit.yaml");
        let lines: Vec<&str> = outcome.stdout.lines().collect();
        assert_eq!(lines.len(), 4, "{}", outcome.stdout);
        assert_eq!(lines[0], format!("Wrote {}", path.display()));
        assert!(lines[1].starts_with("kit.id: blake3:"), "{}", lines[1]);
        assert_eq!(
            lines[2],
            format!("Next: arkavo kit validate {}", path.display())
        );
        assert_eq!(
            lines[3],
            format!("Run:  arkavo agent -c {}", path.display())
        );
    }

    /// Regression: `kit validate a.yaml b.yaml` checked only `a.yaml`.
    #[test]
    fn validate_checks_every_path_and_fails_if_any_fails() {
        let dir = tempfile::tempdir().unwrap();
        let first = init_kit(dir.path(), "first").unwrap().path;
        let second = init_kit(dir.path(), "second").unwrap().path;
        let broken = dir.path().join("broken.swarmkit.yaml");
        std::fs::write(&broken, "not: [valid, yaml: structure").unwrap();
        let missing = dir.path().join("missing.swarmkit.yaml");
        let as_arg = |p: &Path| p.to_string_lossy().into_owned();

        let both_valid = run_in(dir.path(), &["validate", &as_arg(&first), &as_arg(&second)]);
        assert_eq!(both_valid.result, Ok(()));
        assert!(
            both_valid.stdout.contains("kit: first"),
            "{}",
            both_valid.stdout
        );
        assert!(
            both_valid.stdout.contains("kit: second"),
            "{}",
            both_valid.stdout
        );

        let second_bad = run_in(dir.path(), &["validate", &as_arg(&first), &as_arg(&broken)]);
        let err = second_bad.result.unwrap_err();
        assert!(err.contains("broken.swarmkit.yaml"), "{err}");
        assert_eq!(err.lines().count(), 1, "one failure, one line: {err}");

        let first_bad = run_in(
            dir.path(),
            &[
                "validate",
                &as_arg(&broken),
                &as_arg(&second),
                &as_arg(&missing),
            ],
        );
        assert!(
            first_bad.stdout.contains("kit: second"),
            "a failure must not stop later paths being checked: {}",
            first_bad.stdout
        );
        let err = first_bad.result.unwrap_err();
        assert!(err.contains("broken.swarmkit.yaml"), "{err}");
        assert!(err.contains("missing.swarmkit.yaml"), "{err}");
        assert_eq!(err.lines().count(), 2, "one line per failed file: {err}");
    }

    #[test]
    fn validate_of_one_path_keeps_the_unlabelled_report() {
        let dir = tempfile::tempdir().unwrap();
        let path = init_kit(dir.path(), "solo").unwrap().path;
        let outcome = run_in(dir.path(), &["validate", &path.to_string_lossy()]);
        assert_eq!(outcome.result, Ok(()));
        assert!(
            outcome.stdout.starts_with("kit: solo\nkit.id: blake3:"),
            "{}",
            outcome.stdout
        );
    }
}
