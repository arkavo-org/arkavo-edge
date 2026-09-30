//! Auto-approval classification for `shell_exec`.
//!
//! First-word allowlisting is unsafe because the base word alone decided the
//! verdict while the rest of the command — extra pipeline segments, wrappers,
//! interpreter arguments — went unexamined. Classification here reasons about
//! every segment of the pipeline and refuses to bless a command that can hand
//! control to an unlisted program.

use super::args::{arguments_stay_inside, long_option, option_words, short_option};
use super::blocklist::{check_blocklist, check_injection};
use super::git::git_read_only;
use super::redirect::{redirections_within_root, without_redirections};
use std::path::Path;

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
/// segment base, except for the fixed probes in PACKAGE_INFO and
/// VERSION_PROBE_OK.
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
    "-exec",
    "-execdir",
    "-ok",
    "-okdir",
    "-delete",
    "-fprint",
    "-fprintf",
    "-fprint0",
    "-fls",
    "-files0-from",
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

/// Tools whose `--version` only prints a version. A fixed set: any other bare
/// name runs whatever PATH resolves it to (`curl`, a planted binary), and
/// `-v` is not a version flag everywhere.
const VERSION_PROBE_OK: &[&str] = &[
    "cargo", "rustc", "rustup", "node", "npm", "python3", "python", "pip", "go", "java", "javac",
    "clang", "gcc", "make", "cmake", "git", "docker", "kubectl", "deno", "bun",
];

pub(super) fn classify(command: &str, root: &Path, cwd: &Path) -> ApprovalResult {
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
    // sh splits words on space and tab only, so a no-break or ideographic
    // space is part of a word here but a separator to the whitespace-based
    // checks below.
    if cmd_trimmed
        .chars()
        .any(|c| c.is_whitespace() && !matches!(c, ' ' | '\t'))
    {
        return ApprovalResult::RequiresReview;
    }
    if all_segments_safe(cmd_trimmed, root, cwd) {
        return ApprovalResult::AutoApproved;
    }
    ApprovalResult::RequiresReview
}

/// Split on unquoted `|` and require every segment to be a safe read-only
/// command. Subsumes the original single-pipe special case.
fn all_segments_safe(cmd: &str, root: &Path, cwd: &Path) -> bool {
    let segments = split_pipeline(cmd);
    if segments.is_empty() {
        return false;
    }
    segments
        .iter()
        .all(|seg| segment_safe(seg.trim(), root, cwd))
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

fn segment_safe(seg: &str, root: &Path, cwd: &Path) -> bool {
    // A redirection is approvable only when its target is confined to the
    // workspace root.
    if !redirections_within_root(seg, root, cwd) {
        return false;
    }
    // An expansion's value is not in the string: `$KEY` prints the agent's
    // environment and `/pr${X}oc` builds a path no check can read. `%` expands
    // the same way under `cmd /C`.
    // `^` escapes under `cmd /C` and is removed before the program sees it
    // (`-del^ete`), which no string check here follows.
    if seg.contains('$') || (cfg!(windows) && seg.contains(['%', '^'])) {
        return false;
    }
    // Arguments stay inside the working tree: no absolute or home path, no
    // `..`, no glob that can match `..` (this is what keeps `/proc/*/environ`
    // and `~/.ssh` out). Redirection targets were judged against the root.
    // Every check below reads the words the program receives, so it never
    // sees a redirection or its target as a word.
    let seg = &without_redirections(seg);
    if !arguments_stay_inside(seg) {
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
    if is_package_info(seg) || is_version_probe(seg) {
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

    SAFE_BASE.contains(&base.as_str()) && !runs_or_writes(&base, seg)
}

/// Options that turn a safe-listed base into one that runs a program or
/// writes a file: ripgrep's `--pre`/`--pre-glob` preprocessors and
/// `--hostname-bin`, `sort -o`/`--output`/`--compress-program`, `uniq`'s
/// OUTPUT operand, `tree -o`/`-R`, `less`'s log files and `file -C`, which
/// compiles `magic.mgc` into the working directory. `--files0-from` (sort,
/// wc, du) and `file -f` open every name listed in a file, so a planted list
/// reads any path.
fn runs_or_writes(base: &str, seg: &str) -> bool {
    if !matches!(
        base,
        "rg" | "sort" | "uniq" | "tree" | "less" | "file" | "wc" | "du"
    ) {
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
        "wc" | "du" => args.iter().any(|a| long_option(a, "--files0-from")),
        "sort" => args.iter().any(|a| {
            long_option(a, "--files0-from")
                || long_option(a, "--output")
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
        "file" => args.iter().any(|a| {
            long_option(a, "--compile")
                || long_option(a, "--files-from")
                || short_option(a, &['C', 'f'])
        }),
        _ => false,
    }
}

/// `<tool> --version` for a tool in VERSION_PROBE_OK, and nothing more.
fn is_version_probe(seg: &str) -> bool {
    let words: Vec<String> = seg.split_whitespace().map(str::to_lowercase).collect();
    matches!(words.as_slice(), [tool, flag]
        if flag == "--version" && VERSION_PROBE_OK.contains(&tool.as_str()))
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
        let here = std::env::current_dir().unwrap();
        classify(cmd, &here, &here)
    }

    /// A canonical temporary workspace, so targets are judged the way
    /// `resolve_within_root` judges them on disk.
    fn workspace() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        (dir, root)
    }

    #[spec("MCP-015")]
    #[test]
    fn redirection_target_confined_to_root() {
        let (_dir, root) = workspace();
        let at_root = |cmd: &str| classify(cmd, &root, &root);
        let inside = root.join("out.txt");
        assert_eq!(
            at_root(&format!("echo hi > {}", inside.display())),
            ApprovalResult::AutoApproved
        );
        assert_eq!(at_root("echo hi > out.txt"), ApprovalResult::AutoApproved);
        // Glued to a word the operator hides that word from option checks,
        // so it goes to review even when the target is inside.
        assert_eq!(at_root("echo hi>out.txt"), ApprovalResult::RequiresReview);
        assert_eq!(at_root("echo hi 2>out.txt"), ApprovalResult::AutoApproved);
        assert_eq!(at_root("echo hi 1>>out.txt"), ApprovalResult::AutoApproved);
        assert_eq!(at_root("cat < out.txt"), ApprovalResult::AutoApproved);
        assert_eq!(
            at_root("ls -la 2>&1 | grep foo"),
            ApprovalResult::AutoApproved
        );
        assert_eq!(at_root("ls 2>/dev/null"), ApprovalResult::AutoApproved);
        assert_eq!(
            at_root("echo hi > /etc/passwd"),
            ApprovalResult::RequiresReview
        );
        assert_eq!(
            at_root("echo hi>/etc/passwd"),
            ApprovalResult::RequiresReview
        );
        assert_eq!(
            at_root("echo hi > ../escape"),
            ApprovalResult::RequiresReview
        );
        assert_eq!(at_root("cat < /etc/hosts"), ApprovalResult::RequiresReview);
        // Targets the shell expands cannot be judged from the string.
        assert_eq!(
            at_root("echo hi > ~/.bashrc"),
            ApprovalResult::RequiresReview
        );
        assert_eq!(
            at_root("echo hi > $HOME/.bashrc"),
            ApprovalResult::RequiresReview
        );
        assert_eq!(
            at_root("echo hi >&$HOME/.bashrc"),
            ApprovalResult::RequiresReview
        );
        assert_eq!(
            at_root("echo hi > %USERPROFILE%\\x"),
            ApprovalResult::RequiresReview
        );
        assert_eq!(at_root("echo hi >"), ApprovalResult::RequiresReview);
    }

    #[cfg(unix)]
    #[spec("MCP-015")]
    #[test]
    fn redirection_through_a_symlink_is_refused() {
        use std::os::unix::fs::symlink;
        let (_dir, root) = workspace();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.join("escape")).unwrap();
        assert_eq!(
            classify("echo hi > escape/loot", &root, &root),
            ApprovalResult::RequiresReview
        );

        // Relative targets resolve against the directory the command runs in:
        // `link` is a real directory at the root but an outward symlink in `sub`.
        std::fs::create_dir(root.join("link")).unwrap();
        std::fs::create_dir(root.join("sub")).unwrap();
        symlink(outside.path(), root.join("sub").join("link")).unwrap();
        assert_eq!(
            classify("echo hi > link/x", &root, &root),
            ApprovalResult::AutoApproved
        );
        assert_eq!(
            classify("echo hi > link/x", &root, &root.join("sub")),
            ApprovalResult::RequiresReview
        );
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
        // Any expansion is refused, so even a plain `echo $HOME` goes to review.
        assert_eq!(verdict("echo $HOME"), ApprovalResult::RequiresReview);
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
    fn assert_not_approved(cmds: &[&str]) {
        for cmd in cmds {
            assert_ne!(
                verdict(cmd),
                ApprovalResult::AutoApproved,
                "{cmd} must not be auto-approved"
            );
        }
    }

    fn assert_approved(cmds: &[&str]) {
        for cmd in cmds {
            assert_eq!(
                verdict(cmd),
                ApprovalResult::AutoApproved,
                "{cmd} stays auto-approved"
            );
        }
    }

    /// Regression: a `$` expansion printed or opened the agent's environment
    /// (`echo $ANTHROPIC_API_KEY`), and built a `/proc` path the literal check
    /// could not see. No expansion is auto-approved.
    #[spec("MCP-013")]
    #[test]
    fn expansion_cannot_read_the_environment() {
        assert_not_approved(&[
            "echo $ANTHROPIC_API_KEY",
            "echo \"${X}\"",
            "echo ${FAKE_PROVIDER_KEY}",
            "cat $P/$PPID/environ",
            "cat /pr${X}oc/$PPID/environ",
            "grep 'foo$' src/lib.rs",
        ]);
    }

    /// Regression: safe-listed readers took any path, so the agent's keys and
    /// files beside the workspace were auto-approved reads.
    #[spec("MCP-013")]
    #[test]
    fn arguments_cannot_name_paths_outside_the_working_tree() {
        assert_not_approved(&[
            "cat ~/.ssh/id_rsa",
            "cat ~/.aws/credentials",
            "head ~/.ssh/id_rsa",
            "grep . ~/.aws/credentials",
            "less ~/.ssh/id_rsa",
            "cat /etc/passwd",
            "head /etc/passwd",
            "grep . /etc/passwd",
            "less /etc/passwd",
            "cut -d: -f1 /etc/passwd",
            "stat /etc/passwd",
            "cat ../outside",
            "cat ../outside_secret.txt",
            "ls ..",
            "grep -r KEY ./..",
            "cat sub/../../x",
            "cat '/etc/passwd'",
            "cat /etc/passwd>out.txt",
            // Values glued to an option or listed after `=` or `:`.
            "grep -f/etc/passwd x",
            "grep --file=/etc/passwd x",
            "grep -f~/x y",
            "file -m a:/etc/passwd x",
            "cat {..,x}/secret",
        ]);
        assert_approved(&[
            "cat src/lib.rs",
            "cut -d: -f1 f",
            "git log HEAD~1",
            "ls ./src",
        ]);
    }

    /// Regression: a glob reached `/proc` without spelling it, and a glob in a
    /// dot-led component can match `..`.
    #[spec("MCP-013")]
    #[test]
    fn globs_cannot_reach_outside_the_working_tree() {
        assert_not_approved(&[
            "cat /pr?c/self/environ",
            "cat /pro[c]/self/environ",
            "cat .*/secret",
            "ls x/.?",
            "cat .[.]*/secret",
        ]);
        assert_approved(&["ls *.rs", "grep foo src/*.rs", "find . -name '*.rs'"]);
    }

    /// Regression: `git remote show` contacts the remote, and the URL may be
    /// given on the command line.
    #[spec("MCP-013")]
    #[test]
    fn git_remote_show_is_not_auto_approved() {
        assert_not_approved(&[
            "git remote show http://attacker/x.git",
            "git remote show origin",
            "git remote show",
        ]);
        assert_approved(&["git remote"]);
    }

    /// Regression: `git remote -v` and `get-url` print remote URLs, which can
    /// embed a token (`https://user:token@host/repo`).
    #[spec("MCP-013")]
    #[test]
    fn git_remote_urls_are_not_auto_approved() {
        assert_not_approved(&[
            "git remote -v",
            "git remote get-url origin",
            "git remote get-url --all origin",
            "git remote --verbose",
        ]);
    }

    /// Regression: `file -C` compiles the magic file into `magic.mgc` in the
    /// working directory.
    #[spec("MCP-013")]
    #[test]
    fn file_compile_is_not_auto_approved() {
        assert_not_approved(&[
            "file -C -m /etc/passwd",
            "file -C -m magic",
            "file -bC -m magic",
            "file --compile -m magic",
        ]);
        assert_approved(&["file src/lib.rs", "file -b src/lib.rs"]);
    }

    /// Regression: any bare name followed by `--version` or `-v` ran whatever
    /// PATH resolved it to (`curl --version`), and `-v` is not a version flag
    /// everywhere. Only a fixed set of tools, with `--version`, is a probe.
    #[spec("MCP-013")]
    #[test]
    fn version_probe_is_a_fixed_set_with_long_flag() {
        assert_not_approved(&[
            "curl --version",
            "wget -v",
            "curl -v",
            "evilbin --version",
            "node -v",
            "clang -v",
        ]);
        assert_approved(&[
            "rustc --version",
            "rustup --version",
            "git --version",
            "make --version",
            "gcc --version",
        ]);
    }

    /// Regression: an operator glued to a word left the word unchanged for the
    /// option checks, which compare a whole token: `-delete>x` is not
    /// `-delete`, yet the shell runs `find . -delete` and redirects its
    /// (empty) output into `x`.
    #[spec("MCP-015")]
    #[test]
    fn glued_redirection_cannot_hide_an_option() {
        let (_dir, root) = workspace();
        for cmd in [
            "find . -delete>x",
            "find . -exec>x sh -c id {} +",
            "find . -execdir>x id {} +",
            "find . -ok>x rm {} +",
            "find . -fprintf>out.txt fmt",
            "find . -fls>x y",
            "find . -delete>>x",
            "find . -delete<x",
            "sort -o>x f",
            "echo hi\"a\">x",
        ] {
            assert_eq!(
                classify(cmd, &root, &root),
                ApprovalResult::RequiresReview,
                "{cmd} must not be auto-approved"
            );
        }
        // The spaced forms are judged by the option check itself.
        for cmd in [
            "find . -delete > x",
            "find . -exec > x sh -c id {} +",
            "find . -fls > x y",
            "find . -delete 2>x",
        ] {
            assert_eq!(
                classify(cmd, &root, &root),
                ApprovalResult::RequiresReview,
                "{cmd}"
            );
        }
        assert_eq!(
            classify("find . -name x > out.txt", &root, &root),
            ApprovalResult::AutoApproved
        );
        assert_eq!(
            classify("find . -name x 2>/dev/null", &root, &root),
            ApprovalResult::AutoApproved
        );
        assert_eq!(
            classify("ls -la 2>&1 | grep foo", &root, &root),
            ApprovalResult::AutoApproved
        );
    }

    /// Defence in depth: option checks see the words with redirections
    /// removed, so a leading redirection cannot displace the base either.
    #[spec("MCP-015")]
    #[test]
    fn option_checks_ignore_redirection_words() {
        let (_dir, root) = workspace();
        assert_eq!(
            classify("> out.txt find . -delete", &root, &root),
            ApprovalResult::RequiresReview
        );
        assert_eq!(
            classify("> out.txt sort -o x f", &root, &root),
            ApprovalResult::RequiresReview
        );
        assert_eq!(
            classify("> out.txt git branch evil", &root, &root),
            ApprovalResult::RequiresReview
        );
        assert_eq!(
            classify("> out.txt ls", &root, &root),
            ApprovalResult::AutoApproved
        );
    }

    /// Regression: `--files0-from` makes sort, wc and du (and find's
    /// `-files0-from`) open every NUL-separated name in a file, so a planted
    /// list reads anything; `file -f` does the same with newline-separated
    /// names.
    #[spec("MCP-013")]
    #[test]
    fn name_list_options_are_not_auto_approved() {
        for cmd in [
            "sort --files0-from=list",
            "sort --files0-from list",
            "sort --files0 list",
            "wc --files0-from=list",
            "du --files0-from=list",
            "find . -files0-from list",
            "file -f list",
            "file -bf list",
            "file --files-from list",
        ] {
            assert_eq!(
                verdict(cmd),
                ApprovalResult::RequiresReview,
                "{cmd} must not be auto-approved"
            );
        }
        assert_eq!(verdict("wc -l notes.txt"), ApprovalResult::AutoApproved);
        assert_eq!(verdict("du -sh ."), ApprovalResult::AutoApproved);
        assert_eq!(verdict("file notes.txt"), ApprovalResult::AutoApproved);
    }

    /// Regression: `git config --list` and `--get-regexp` print every
    /// configured value, including remote URLs carrying tokens and
    /// `http.extraheader`.
    #[spec("MCP-013")]
    #[test]
    fn git_config_is_never_auto_approved() {
        for cmd in [
            "git config --list",
            "git config --get user.name",
            "git config --get-regexp .",
            "git config --get-all remote.origin.url",
        ] {
            assert_eq!(
                verdict(cmd),
                ApprovalResult::RequiresReview,
                "{cmd} must not be auto-approved"
            );
        }
    }

    /// sh splits words on space and tab only; a no-break space or other
    /// Unicode space is part of a word, which the whitespace-splitting checks
    /// would treat as a separator.
    #[spec("MCP-013")]
    #[test]
    fn non_ascii_whitespace_is_not_auto_approved() {
        for space in ['\u{a0}', '\u{2003}', '\u{3000}', '\u{2028}'] {
            let cmd = format!("find .{space}-delete");
            assert_eq!(verdict(&cmd), ApprovalResult::RequiresReview, "{cmd:?}");
        }
    }

    #[cfg(windows)]
    #[spec("MCP-013")]
    #[test]
    fn caret_is_not_auto_approved_under_cmd() {
        assert_eq!(verdict("find . -del^ete"), ApprovalResult::RequiresReview);
    }
}
