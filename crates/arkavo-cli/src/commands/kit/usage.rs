//! Help text for `arkavo kit` and each of its subcommands.

pub(super) const KIT: &str = "\
Arkavo Kit - Author, validate, and migrate SwarmKit manifests

USAGE:
    arkavo kit init <name>
    arkavo kit validate <path>...
    arkavo kit apply <path> [--days <n>]
    arkavo kit migrate-from-agents-md --in <path> --out <path>

SUBCOMMANDS:
    init <name>                                   Write a minimal single-role kit to .arkavo/<name>.swarmkit.yaml
    validate <path>...                            Check that each kit file can be run
    apply <path> [--days <n>]                     Give the kit's roles to agents running on the network
    migrate-from-agents-md --in <in> --out <out>  Best-effort convert an AGENTS.md file into a kit
    help                                          Print this help message

Run 'arkavo kit <subcommand> --help' for details on one subcommand.
Start an agent from a kit with 'arkavo agent -c <path>', or give its roles to
running agents with 'arkavo kit apply <path>'.
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

A valid kit may still declare settings that 'arkavo agent -c <path>' does not
enforce, such as isolation, budgets and network constraints. These are listed
in a notice after the result and do not affect the exit status.

Every path is checked. The exit status is non-zero if any file fails.
";

pub(super) const APPLY: &str = "\
Give a kit's roles to agents already running on the network

USAGE:
    arkavo kit apply <path> [--days <n>]

OPTIONS:
    --days <n>    How long the agents hold their roles, 1 to 30 days (default 7)

The kit is validated first and must have its kit.id set. Agents are found
over mDNS, and each role goes to one agent that can take it: roles that need
a host capability, such as image_generation, go first, to a host that has
it ready if one does. A role an agent refuses is offered to the next one.
If a role finds no agent, the roles already given out are taken back and
nothing changes.

The orchestrator is this host's agent identity key, printed first. An agent
started without a kit binds to the first orchestrator that gives it a role
and refuses every other afterwards; start it with
'arkavo agent --orchestrator <did:key>' to choose, and release it with
'arkavo agent release'. Agents keep their roles across restarts until they
expire. An agent started with a kit, and a pipeline kit, are not used:
start those roles with 'arkavo agent -c <path> -n <role>'.
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
