//! `arkavo security`: the command line of the security audit.
//!
//! The audit reads configuration and reports on it. It starts no server and
//! loads no model, so it is safe to run anywhere, a CI job included.

use crate::commands::agent::listen::{BindAddress, parse_bind};
use crate::commands::security_audit;

pub(crate) fn help_line() -> &'static str {
    "    security audit Report on the security posture of this configuration"
}

const HELP: &str = "Report on the security posture of this configuration

USAGE:
    arkavo security audit [OPTIONS]

Checks the configuration `arkavo agent` would run with in the current
directory: file permissions, the RPC endpoint (listen address, transport,
authentication, rate limiting), preflight moderation, memory encryption, the
kit and the shell command policy. Nothing is started or changed.

Without --bind the audit describes an agent started without it. By default
that agent listens on every interface, and its RPC endpoint is
not authenticated, so the endpoint checks fail.

Exits with status 1 when a check fails.

COMMANDS:
    audit    Run every check and print the report

OPTIONS:
    --bind <ADDRESS>  Audit the agent as started with --bind <ADDRESS>, for
                      example --bind 127.0.0.1 for an agent kept on this machine
    --json            Print the report as JSON
    -h, --help        Show this help";

/// What an invocation asks for, decided from its arguments alone.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    Help,
    Audit {
        json: bool,
        bind: Option<BindAddress>,
    },
}

fn usage_error(message: &str) -> String {
    format!("{message}\nRun 'arkavo security --help' for usage")
}

/// Anything unrecognized is refused rather than ignored: a mistyped `--json`
/// would otherwise hand a text report to the program that asked for JSON.
fn plan(args: &[String]) -> Result<Action, String> {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        return Ok(Action::Help);
    }
    let Some((subcommand, options)) = args.split_first() else {
        return Err(usage_error("missing subcommand for 'arkavo security'"));
    };
    match subcommand.as_str() {
        "help" => return Ok(Action::Help),
        "audit" => {}
        other => {
            return Err(usage_error(&format!(
                "unknown security subcommand '{other}'"
            )));
        }
    }

    let mut json = false;
    let mut bind = None;
    let mut options = options.iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "--json" => json = true,
            // The agent refuses a bind it cannot understand, so the audit
            // has nothing to describe for one.
            "--bind" => match options.next() {
                Some(value) => bind = Some(parse_bind(value).map_err(|err| usage_error(&err))?),
                None => {
                    return Err(usage_error(
                        "--bind requires an address for 'arkavo security audit', for example \
                         --bind 127.0.0.1",
                    ));
                }
            },
            other => {
                return Err(usage_error(&format!(
                    "unexpected argument '{other}' for 'arkavo security audit'"
                )));
            }
        }
    }
    Ok(Action::Audit { json, bind })
}

pub(crate) fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    match plan(args)? {
        Action::Help => {
            println!("{HELP}");
            Ok(())
        }
        Action::Audit { json, bind } => match security_audit::execute(json, bind) {
            0 => Ok(()),
            _ => Err("the security audit found failures".into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    #[test]
    fn audit_runs_as_text_unless_json_is_asked_for() {
        assert_eq!(
            plan(&args(&["audit"])),
            Ok(Action::Audit {
                json: false,
                bind: None
            })
        );
        assert_eq!(
            plan(&args(&["audit", "--json"])),
            Ok(Action::Audit {
                json: true,
                bind: None
            })
        );
    }

    /// An audit describes a start without `--bind` unless it is told
    /// otherwise: `--bind` is a flag of the run, not part of the kit.
    #[test]
    fn audit_describes_a_bound_start_only_when_asked() {
        let loopback = parse_bind("127.0.0.1").unwrap();
        assert_eq!(
            plan(&args(&["audit", "--bind", "127.0.0.1"])),
            Ok(Action::Audit {
                json: false,
                bind: Some(loopback)
            })
        );
        assert_eq!(
            plan(&args(&["audit", "--json", "--bind", "127.0.0.1"])),
            Ok(Action::Audit {
                json: true,
                bind: Some(loopback)
            })
        );
        assert_eq!(
            plan(&args(&["audit", "--bind", "[::1]:8342", "--json"])),
            Ok(Action::Audit {
                json: true,
                bind: Some(parse_bind("[::1]:8342").unwrap())
            })
        );
    }

    /// `--trust` no longer says anything about where the agent listens, so
    /// the audit does not take it.
    #[test]
    fn a_bind_the_agent_would_refuse_is_refused_here_too() {
        for (invocation, complaint) in [
            (
                args(&["audit", "--trust"]),
                "unexpected argument '--trust' for 'arkavo security audit'".to_string(),
            ),
            (
                args(&["audit", "--bind"]),
                "--bind requires an address for 'arkavo security audit', for example \
                 --bind 127.0.0.1"
                    .to_string(),
            ),
            (
                args(&["audit", "--bind", "localhost"]),
                parse_bind("localhost").unwrap_err(),
            ),
        ] {
            let err = plan(&invocation).unwrap_err();
            let lines: Vec<&str> = err.lines().collect();
            assert_eq!(
                lines,
                [complaint.as_str(), "Run 'arkavo security --help' for usage"],
                "{invocation:?}"
            );
        }
    }

    #[test]
    fn help_is_recognized_at_both_levels() {
        for invocation in [
            args(&["--help"]),
            args(&["-h"]),
            args(&["help"]),
            args(&["audit", "--help"]),
            args(&["audit", "--json", "-h"]),
        ] {
            assert_eq!(plan(&invocation), Ok(Action::Help), "{invocation:?}");
        }
    }

    #[test]
    fn anything_else_is_refused_with_a_pointer_to_help() {
        for (invocation, complaint) in [
            (args(&[]), "missing subcommand for 'arkavo security'"),
            (args(&["scan"]), "unknown security subcommand 'scan'"),
            (
                args(&["audit", "--jsno"]),
                "unexpected argument '--jsno' for 'arkavo security audit'",
            ),
            (
                args(&["audit", "extra"]),
                "unexpected argument 'extra' for 'arkavo security audit'",
            ),
        ] {
            let err = plan(&invocation).unwrap_err();
            let lines: Vec<&str> = err.lines().collect();
            assert_eq!(
                lines,
                [complaint, "Run 'arkavo security --help' for usage"],
                "{invocation:?}"
            );
        }
    }

    #[test]
    fn help_documents_the_subcommand_and_every_option() {
        for word in [
            "audit",
            "--json",
            "--bind 127.0.0.1",
            "--help",
            "status 1",
            "every interface",
            "not authenticated",
        ] {
            assert!(HELP.contains(word), "help should mention {word}");
        }
        assert!(!HELP.contains("--trust"), "{HELP}");
    }
}
