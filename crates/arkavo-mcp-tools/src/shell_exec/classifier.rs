//! Auto-approval classification for `shell_exec`.
//!
//! First-word allowlisting is unsafe because the base word alone decided the
//! verdict while the rest of the command — extra pipeline segments, wrappers,
//! interpreter arguments — went unexamined. Classification here reasons about
//! every segment of the pipeline and refuses to bless a command that can hand
//! control to an unlisted program.

use super::blocklist::{check_blocklist, check_injection};

/// Result of command classification for auto-approval
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalResult {
    /// Command is safe and auto-approved (read-only operations)
    AutoApproved,
    /// Command is dangerous and auto-blocked
    AutoBlocked(String),
    /// Command requires manual review
    RequiresReview,
}

/// Read-only base commands safe to auto-approve. `env`/`printenv` are removed:
/// `env` launches another program and `printenv`/`env` dump the inherited
/// process environment, which may hold provider credentials.
const SAFE_BASE: &[&str] = &[
    "ls", "cat", "head", "tail", "grep", "egrep", "fgrep", "rg", "pwd", "echo", "wc", "date",
    "whoami", "hostname", "uname", "which", "whereis", "type", "file", "stat", "df", "du", "free",
    "uptime", "id", "groups", "tree", "less", "more", "sort", "uniq", "cut", "nl", "basename",
    "dirname", "find",
];

/// Programs that launch or become another command. Never auto-approved as a
/// segment base, and never eligible for the version-flag shortcut.
const EXEC_DENY: &[&str] = &[
    // interpreters
    "sh",
    "bash",
    "dash",
    "zsh",
    "ksh",
    "csh",
    "tcsh",
    "fish",
    "python",
    "python2",
    "python3",
    "node",
    "nodejs",
    "deno",
    "bun",
    "ruby",
    "perl",
    "php",
    "lua",
    "rscript",
    "osascript",
    "pwsh",
    "powershell",
    "awk",
    "gawk",
    "mawk",
    "sed",
    "expect",
    "tclsh",
    // exec wrappers / launchers
    "env",
    "printenv",
    "nohup",
    "time",
    "nice",
    "ionice",
    "timeout",
    "xargs",
    "watch",
    "stdbuf",
    "setsid",
    "unbuffer",
    "script",
    "eval",
    "exec",
    "command",
    "doas",
    "chroot",
    "flock",
    "parallel",
    "at",
    "batch",
    "make",
    "cmake",
];

/// `find` predicates that execute or mutate. Their presence disqualifies the
/// otherwise read-only `find`.
const FIND_ACTIONS: &[&str] = &[
    "-exec", "-execdir", "-ok", "-okdir", "-delete", "-fprint", "-fprintf", "-fprint0", "-fls",
];

/// Git subcommand prefixes that only read. `starts_with` on the raw command is
/// not enough (it also matched `git log --output=`), so the whole token stream
/// must match one of these and carry no `-c`/`--output` argument.
const GIT_READ_SUBCMDS: &[&str] = &[
    "status",
    "log",
    "diff",
    "branch",
    "remote",
    "show",
    "describe",
    "rev-parse",
    "ls-files",
    "ls-tree",
    "cat-file",
    "config",
];

/// `git branch` flags that only list. A bare name creates a branch, so every
/// argument must be one of these.
const GIT_BRANCH_READ_FLAGS: &[&str] = &[
    "-a",
    "-r",
    "-v",
    "-vv",
    "-l",
    "--list",
    "--all",
    "--remotes",
    "--verbose",
    "--show-current",
    "--no-color",
    "--merged",
    "--no-merged",
];

/// Fixed package-manager and toolchain info commands (read-only), from the
/// original allowlist. Matched on the whole segment, so nothing can be
/// appended to them.
const PACKAGE_INFO: &[&str] = &[
    "cargo --version",
    "cargo version",
    "rustc --version",
    "npm --version",
    "npm list",
    "npm ls",
    "node --version",
    "python --version",
    "python3 --version",
    "pip list",
    "pip3 list",
    "go version",
    "java -version",
    "java --version",
    "ruby --version",
    "gem list",
];

pub(super) fn classify(command: &str) -> ApprovalResult {
    let cmd_lower = command.to_lowercase();
    let cmd_trimmed = command.trim();

    if let Some(reason) = check_blocklist(&cmd_lower) {
        return ApprovalResult::AutoBlocked(reason);
    }
    if let Some(reason) = check_injection(cmd_trimmed) {
        return ApprovalResult::AutoBlocked(reason);
    }
    // sh drops a backslash and keeps the next character literal, which the
    // string checks below do not follow: `\"` would open a quote that hides a
    // later `|`, and `-\exec` would pass as a harmless word. A command the
    // checks cannot read the way the shell does is not approved.
    if cfg!(not(windows)) && cmd_trimmed.contains('\\') {
        return ApprovalResult::RequiresReview;
    }
    if all_segments_safe(cmd_trimmed) {
        return ApprovalResult::AutoApproved;
    }
    ApprovalResult::RequiresReview
}

/// Split on unquoted `|` and require every segment to be a safe read-only
/// command. Subsumes the original single-pipe special case.
fn all_segments_safe(cmd: &str) -> bool {
    let segments = split_pipeline(cmd);
    if segments.is_empty() {
        return false;
    }
    segments.iter().all(|seg| segment_safe(seg.trim()))
}

/// Quote-aware split on `|`. A `|` inside single or double quotes (e.g. a grep
/// regex) is not a pipeline separator.
fn split_pipeline(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in cmd.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
                cur.push(c);
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    cur.push(c);
                }
                '|' => {
                    out.push(std::mem::take(&mut cur));
                }
                _ => cur.push(c),
            },
        }
    }
    out.push(cur);
    out
}

fn segment_safe(seg: &str) -> bool {
    // Any redirection downgrades to review here; confining its target to the
    // workspace root is what makes it safe to approve.
    if seg.contains('>') || seg.contains('<') {
        return false;
    }
    // `/proc/<pid>/environ` of the `sh -c` parent is the agent's own
    // environment, provider keys included, and same-UID reads are allowed.
    // Quotes are removed first so `/pr''oc` is caught, but a string check
    // stops literal spellings only (a glob such as `/pro[c]` still reaches
    // it); the broker holding the keys closes it.
    if seg.replace(['\'', '"'], "").contains("/proc") {
        return false;
    }
    let Some(base_raw) = seg.split_whitespace().next() else {
        return false;
    };
    let base = base_raw.to_lowercase();

    // A base with a path separator is not a bare, resolvable tool name.
    if base.contains('/') || base.contains('\\') || base.starts_with('.') {
        return false;
    }
    // Before the interpreter list: `python --version` only prints a version.
    if is_package_info(seg) {
        return true;
    }
    if EXEC_DENY.contains(&base.as_str()) {
        return false;
    }

    if base == "git" {
        return option_words(seg).is_some_and(|words| git_read_only(&words));
    }
    if base == "find" {
        return option_words(seg).is_some_and(|words| {
            !words
                .iter()
                .any(|t| FIND_ACTIONS.contains(&t.to_lowercase().as_str()))
        });
    }

    if SAFE_BASE.contains(&base.as_str()) {
        return !runs_or_writes(&base, seg);
    }
    is_version_probe(base_raw, seg)
}

/// Options that turn a safe-listed base into one that runs a program or
/// writes a file: ripgrep's `--pre`/`--pre-glob` preprocessors and
/// `--hostname-bin`, `sort -o`/`--output`/`--compress-program`, `uniq`'s
/// OUTPUT operand, `tree -o`/`-R` and `less`'s log files.
fn runs_or_writes(base: &str, seg: &str) -> bool {
    if !matches!(base, "rg" | "sort" | "uniq" | "tree" | "less") {
        return false;
    }
    let Some(words) = option_words(seg) else {
        return true;
    };
    let args = &words[1..];
    match base {
        "rg" => args.iter().any(|a| {
            long_option(a, "--pre")
                || long_option(a, "--pre-glob")
                || long_option(a, "--hostname-bin")
        }),
        "sort" => args.iter().any(|a| {
            long_option(a, "--output")
                || long_option(a, "--compress-program")
                || short_option(a, &['o'])
        }),
        "uniq" => args.iter().filter(|a| !a.starts_with('-')).count() > 1,
        "tree" => args.iter().any(|a| short_option(a, &['o', 'R'])),
        "less" => args.iter().any(|a| {
            long_option(a, "--log-file")
                || long_option(a, "--LOG-FILE")
                || short_option(a, &['o', 'O'])
        }),
        _ => false,
    }
}

/// The words of `seg` as the program receives them, for the bases whose
/// options are checked. Quotes are removed, so `-e''xec` is judged as
/// `-exec` (a backslash never gets here; `classify` refuses it). `None` when
/// a word can still change at run time: a `$` expansion, whose value the
/// caller's env map can choose, or a bash brace expansion such as `-{exec,}`.
fn option_words(seg: &str) -> Option<Vec<String>> {
    seg.split_whitespace()
        .map(|word| {
            let braces = word.contains('{') && (word.contains(',') || word.contains(".."));
            (!word.contains('$') && !braces).then(|| word.replace(['\'', '"'], ""))
        })
        .collect()
}

/// Whether `arg` is the long option `name`, or a GNU-style abbreviation of it
/// (getopt_long accepts any unambiguous prefix), with or without `=value`.
fn long_option(arg: &str, name: &str) -> bool {
    let given = arg.split('=').next().unwrap_or(arg);
    given.len() > 2 && given.starts_with("--") && name.starts_with(given)
}

/// Whether `arg` is a cluster of short options (`-nro`) containing one of
/// `letters`.
fn short_option(arg: &str, letters: &[char]) -> bool {
    arg.len() > 1
        && arg.starts_with('-')
        && !arg.starts_with("--")
        && arg[1..].chars().any(|c| letters.contains(&c))
}

/// `<tool> --version` / `-v` / `-V`, exactly two tokens, tool a bare name.
/// `segment_safe` has already refused paths and `EXEC_DENY` names.
fn is_version_probe(base_raw: &str, seg: &str) -> bool {
    let toks: Vec<&str> = seg.split_whitespace().collect();
    if toks.len() != 2 {
        return false;
    }
    let flag = toks[1].to_lowercase();
    if !matches!(flag.as_str(), "--version" | "-v") {
        return false;
    }
    base_raw
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Read-only git: subcommand in GIT_READ_SUBCMDS, no top-level `-c` config
/// override and no output-redirecting flag; `config`/`branch`/`remote` must
/// stay in their read forms. The read flag must lead: `git config` takes a
/// later `--get` as a value pattern, and `git remote -v` a subcommand.
fn git_read_only(toks: &[String]) -> bool {
    // toks[0] == "git"; a global option such as -C or --git-dir in toks[1] is
    // not a read subcommand, so it is refused here too.
    let Some(sub) = toks.get(1) else { return false };
    let sub = sub.to_lowercase();
    if !GIT_READ_SUBCMDS.contains(&sub.as_str()) {
        return false;
    }
    if toks.iter().any(|t| {
        let t = t.to_lowercase();
        t == "-c" || t.starts_with("--output") || t.starts_with("-o=") || t == "--exec-path"
    }) {
        return false;
    }
    match sub.as_str() {
        "branch" => toks[2..]
            .iter()
            .all(|t| GIT_BRANCH_READ_FLAGS.contains(&t.as_str())),
        "remote" => match toks.get(2).map(|s| s.to_lowercase()).as_deref() {
            None | Some("show" | "get-url") => true,
            Some("-v") => toks.len() == 3,
            _ => false,
        },
        "config" => matches!(
            toks.get(2).map(String::as_str),
            Some("--list" | "--get" | "--get-all" | "--get-regexp")
        ),
        _ => true,
    }
}

/// Whether the segment is exactly one of the fixed info commands.
fn is_package_info(seg: &str) -> bool {
    let words: Vec<String> = seg.split_whitespace().map(str::to_lowercase).collect();
    PACKAGE_INFO.iter().any(|known| {
        known
            .split_whitespace()
            .eq(words.iter().map(String::as_str))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    /// Classification of a command with no redirection, where the workspace
    /// root plays no part.
    fn verdict(cmd: &str) -> ApprovalResult {
        classify(cmd)
    }

    #[spec("MCP-013")]
    #[test]
    fn every_pipeline_segment_must_be_safe() {
        // A safe first word must not launch a shell, interpreter or arbitrary
        // command later in the pipeline or after a wrapper.
        assert_eq!(verdict("cat x | sh"), ApprovalResult::RequiresReview);
        assert_eq!(verdict("cat x|sh"), ApprovalResult::RequiresReview);
        assert_eq!(verdict("env bash -c 'id'"), ApprovalResult::RequiresReview);
        assert_eq!(verdict("env nohup sh"), ApprovalResult::RequiresReview);
        assert_eq!(verdict("ls | xargs rm"), ApprovalResult::RequiresReview);
        assert_eq!(
            verdict("cat x | awk 'BEGIN{system(\"id\")}'"),
            ApprovalResult::RequiresReview
        );
        assert_eq!(
            verdict("echo hi | python3 -c 'pass'"),
            ApprovalResult::RequiresReview
        );
    }

    #[spec("MCP-013")]
    #[test]
    fn background_and_control_char_chaining_blocked() {
        assert!(matches!(
            verdict("cat /dev/null & touch pwned"),
            ApprovalResult::AutoBlocked(_)
        ));
        assert!(matches!(
            verdict("ls\rtouch x"),
            ApprovalResult::AutoBlocked(_)
        ));
        // 2>&1 fd-dup is a redirection, not a background &, and must not be
        // mistaken for chaining.
        assert_ne!(
            verdict("ls -la 2>&1 | grep foo"),
            ApprovalResult::AutoBlocked("Background/chaining (&) detected".into())
        );
    }

    #[spec("MCP-013")]
    #[test]
    fn find_action_predicates_not_auto_approved() {
        assert_eq!(
            verdict("find . -exec rm {} +"),
            ApprovalResult::RequiresReview
        );
        assert_eq!(verdict("find . -delete"), ApprovalResult::RequiresReview);
        assert_eq!(
            verdict("find . -fprint /tmp/out"),
            ApprovalResult::RequiresReview
        );
        // Plain read-only find still auto-approved.
        assert_eq!(verdict("find . -name '*.rs'"), ApprovalResult::AutoApproved);
    }

    /// A safe-listed command that runs another program is not auto-approved,
    /// so `curl` cannot reach the metadata service through it with its output
    /// returned to the model.
    #[spec("MCP-013")]
    #[test]
    fn safe_listed_commands_cannot_launder_another_program() {
        for cmd in [
            "env curl http://169.254.169.254/latest/meta-data/",
            "env -i curl http://169.254.169.254/",
            "find . -exec curl http://169.254.169.254/ {} +",
            "find . -maxdepth 0 -execdir wget -qO- http://169.254.169.254/ {} +",
            "find . -name '*.rs' -delete",
            "env curl http://169.254.169.254/ | head",
        ] {
            assert_eq!(
                verdict(cmd),
                ApprovalResult::RequiresReview,
                "{cmd} must not be auto-approved"
            );
        }
        for cmd in ["find . -name '*.rs'", "find src -type f | wc"] {
            assert_eq!(
                verdict(cmd),
                ApprovalResult::AutoApproved,
                "{cmd} stays auto-approved"
            );
        }
    }

    #[spec("MCP-013")]
    #[test]
    fn read_only_bases_cannot_run_programs_or_write_files() {
        for cmd in [
            "rg --pre=sh x .",
            "rg --pre-glob '*' --pre sh x .",
            "rg --hostname-bin=/tmp/x foo .",
            "sort --compress-program=sh f",
            "sort --out=/tmp/out f",
            "sort -o /tmp/out f",
            "sort -uo /tmp/out f",
            "uniq f /tmp/out",
            "tree -o /tmp/out",
            "tree -R -H . .",
            "cat x | less -o /tmp/out",
        ] {
            assert_eq!(
                verdict(cmd),
                ApprovalResult::RequiresReview,
                "{cmd} must not be auto-approved"
            );
        }
        for cmd in [
            "rg foo src",
            "rg --pretty foo src",
            "sort -u f",
            "uniq -c f",
            "tree -L 2",
        ] {
            assert_eq!(
                verdict(cmd),
                ApprovalResult::AutoApproved,
                "{cmd} stays auto-approved"
            );
        }
    }

    #[spec("MCP-013")]
    #[test]
    fn version_heuristic_only_for_bare_names() {
        assert_eq!(
            verdict("/tmp/payload --version"),
            ApprovalResult::RequiresReview
        );
        assert_eq!(verdict("./evil -v"), ApprovalResult::RequiresReview);
        assert_eq!(verdict("bash -v"), ApprovalResult::RequiresReview);
        assert_eq!(verdict("python3 -v"), ApprovalResult::RequiresReview);
        assert_eq!(
            verdict("python3 --version -c 'import os'"),
            ApprovalResult::RequiresReview
        );
        // Legitimate tool version probes still work, including the fixed
        // interpreter probes on the package-info list.
        assert_eq!(verdict("cargo --version"), ApprovalResult::AutoApproved);
        assert_eq!(verdict("clang --version"), ApprovalResult::AutoApproved);
        assert_eq!(verdict("node --version"), ApprovalResult::AutoApproved);
        assert_eq!(verdict("python --version"), ApprovalResult::AutoApproved);
    }

    #[spec("MCP-013")]
    #[test]
    fn environment_dump_not_auto_approved() {
        assert_eq!(verdict("printenv"), ApprovalResult::RequiresReview);
        assert_eq!(verdict("env"), ApprovalResult::RequiresReview);
        assert_eq!(verdict("env | grep path"), ApprovalResult::RequiresReview);
        assert_eq!(verdict("printenv HOME"), ApprovalResult::RequiresReview);
        // The agent's own environment, read through the parent's /proc entry.
        assert_eq!(
            verdict("cat /proc/$PPID/environ"),
            ApprovalResult::RequiresReview
        );
        assert_eq!(
            verdict("grep -a KEY /proc/self/environ"),
            ApprovalResult::RequiresReview
        );
    }

    #[spec("MCP-013")]
    #[test]
    fn git_writes_and_pager_injection_not_auto_approved() {
        assert_eq!(
            verdict("git branch -D main"),
            ApprovalResult::RequiresReview
        );
        assert_eq!(verdict("git branch evil"), ApprovalResult::RequiresReview);
        assert_eq!(
            verdict("git remote add evil https://x.invalid/r.git"),
            ApprovalResult::RequiresReview
        );
        assert_eq!(
            verdict("git log --output=/tmp/out"),
            ApprovalResult::RequiresReview
        );
        assert_eq!(
            verdict("git -c core.pager=id log"),
            ApprovalResult::RequiresReview
        );
        assert_eq!(
            verdict("git -C /tmp/other status"),
            ApprovalResult::RequiresReview
        );
        // Read operations still auto-approved.
        assert_eq!(verdict("git status"), ApprovalResult::AutoApproved);
        assert_eq!(verdict("git log --oneline"), ApprovalResult::AutoApproved);
        assert_eq!(verdict("git branch -a"), ApprovalResult::AutoApproved);
    }

    /// Regression: a read flag anywhere in the words let a write through.
    /// `git config` stops parsing options at the key, so a trailing `--get`
    /// is a value pattern and the key is set, and `git remote -v` accepts a
    /// subcommand after `-v`. Both wrote `.git/config` when run, which a
    /// later auto-approved `git status` or `git diff` then obeys
    /// (`core.fsmonitor`, `diff.external`).
    #[spec("MCP-013")]
    #[test]
    fn git_read_flag_cannot_carry_a_config_write() {
        for cmd in [
            "git config user.name evil --get",
            "git config core.fsmonitor ./x --list",
            "git remote -v add evil https://x.invalid/r.git",
            "git remote -v rename a b",
        ] {
            assert_eq!(
                verdict(cmd),
                ApprovalResult::RequiresReview,
                "{cmd} must not be auto-approved"
            );
        }
        for cmd in [
            "git remote -v",
            "git config --get user.name",
            "git config --list",
        ] {
            assert_eq!(
                verdict(cmd),
                ApprovalResult::AutoApproved,
                "{cmd} stays auto-approved"
            );
        }
    }

    /// Regression: `sh` drops a backslash and keeps the next character, so
    /// `\"` desynchronised the quote-aware pipe split and `-\exec` passed
    /// the predicate check; both were auto-approved and ran. The rule is
    /// unix-only because `\` is a path separator for `cmd /C`.
    #[cfg(unix)]
    #[spec("MCP-013")]
    #[test]
    fn backslash_cannot_hide_a_pipe_or_a_flag() {
        for cmd in [r#"echo \" | sh -c id \""#, r#"find . -\exec sh -c id {} +"#] {
            assert_eq!(
                verdict(cmd),
                ApprovalResult::RequiresReview,
                "{cmd} must not be auto-approved"
            );
        }
    }

    /// Regression: the shell removes quotes and expands `$` before the
    /// program sees its arguments, so a flag hidden that way was auto-approved
    /// and ran (`-e''xec`, and `$A` with `A=-exec` from the caller's env map).
    #[spec("MCP-013")]
    #[test]
    fn quoting_and_expansion_cannot_hide_a_flag() {
        for cmd in [
            r#"find . -e''xec sh -c id {} +"#,
            r#"find . $A sh -c id {} +"#,
            r#"find . -{exec,} sh -c id {} +"#,
            r#"rg --pr""e=sh x ."#,
            r#"sort -'o' /tmp/out f"#,
            r#"git log --'output'=/tmp/out"#,
            r#"cat /pr''oc/self/environ"#,
        ] {
            assert_eq!(
                verdict(cmd),
                ApprovalResult::RequiresReview,
                "{cmd} must not be auto-approved"
            );
        }
        // Quoted arguments that name no option stay approved.
        assert_eq!(verdict("find . -name '*.rs'"), ApprovalResult::AutoApproved);
        assert_eq!(verdict("echo $HOME"), ApprovalResult::AutoApproved);
    }

    /// Regression: a bare `version` operand is not a version flag. `crontab
    /// version` installs the file `version` as a crontab and `source version`
    /// runs it; main sent both to review.
    #[spec("MCP-013")]
    #[test]
    fn version_word_is_not_a_version_probe() {
        for cmd in ["crontab version", "source version", "rm version"] {
            assert_eq!(
                verdict(cmd),
                ApprovalResult::RequiresReview,
                "{cmd} must not be auto-approved"
            );
        }
        for cmd in ["go version", "cargo version", "java -version"] {
            assert_eq!(
                verdict(cmd),
                ApprovalResult::AutoApproved,
                "{cmd} stays auto-approved"
            );
        }
    }

    #[spec("MCP-013")]
    #[test]
    fn legitimate_read_only_commands_still_approved() {
        assert_eq!(verdict("ls -la"), ApprovalResult::AutoApproved);
        assert_eq!(
            verdict("ls -la | grep foo | wc -l"),
            ApprovalResult::AutoApproved
        );
        assert_eq!(
            verdict("cat Cargo.toml | head -20"),
            ApprovalResult::AutoApproved
        );
        assert_eq!(verdict("grep -r \"a|b\" ."), ApprovalResult::AutoApproved); // pipe inside quotes
        assert_eq!(verdict("git diff HEAD"), ApprovalResult::AutoApproved);
    }
}
