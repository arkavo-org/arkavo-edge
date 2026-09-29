//! Help text for `arkavo kit` and each of its subcommands.

pub(super) const KIT: &str = "\
Arkavo Kit - Author, validate, and migrate SwarmKit manifests

USAGE:
    arkavo kit init <name>
    arkavo kit validate <path>...
    arkavo kit migrate-from-agents-md --in <path> --out <path>

SUBCOMMANDS:
    init <name>                                   Write a minimal single-role kit to .arkavo/<name>.swarmkit.yaml
    validate <path>...                            Check that each kit file can be run
    migrate-from-agents-md --in <in> --out <out>  Best-effort convert an AGENTS.md file into a kit
    help                                          Print this help message

Run 'arkavo kit <subcommand> --help' for details on one subcommand.
Start an agent from a kit with 'arkavo agent -c <path>'.
";

pub(super) const INIT: &str = "\
Write a minimal single-role kit to .arkavo/<name>.swarmkit.yaml

USAGE:
    arkavo kit init <name>

ARGS:
    <name>    Kit name, also used as the file name. It cannot contain '/', '\\'
              or '..', and cannot be one of the reserved words: help, init,
              validate, migrate-from-agents-md.

An existing file is never overwritten.
Check the new kit with 'arkavo kit validate <path>' and start it with
'arkavo agent -c <path>'.
";

pub(super) const VALIDATE: &str = "\
Check that each kit file can be run by 'arkavo agent -c <path>'

USAGE:
    arkavo kit validate <path>...

Each file must parse, satisfy the cross-block rules, declare a kit.id that
matches its content hash, not be past kit.expires, and name only models the
router knows. An empty kit.id is accepted while a kit is being authored; the
computed id is printed so it can be set.

Every path is checked. The exit status is non-zero if any file fails.
";

pub(super) const MIGRATE: &str = "\
Best-effort convert an AGENTS.md file into a kit

USAGE:
    arkavo kit migrate-from-agents-md --in <path> --out <path>

OPTIONS:
    --in <path>     AGENTS.md file to read
    --out <path>    Kit file to write

Fields with no place in a kit are listed on stderr and make the exit status
non-zero; the kit file is still written.
";
