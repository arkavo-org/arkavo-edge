//! Which `git` invocations are read-only.

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

/// Read-only git: subcommand in GIT_READ_SUBCMDS, no top-level `-c` config
/// override and no output-redirecting flag; `branch`/`remote` must stay in
/// their read forms. `git remote` passes only bare, listing names: `-v` and
/// `get-url` print remote URLs, `show` contacts the remote. `git config` is
/// never read-only here. Both `git config --list`/`--get-regexp` and remote
/// URLs can print credential-bearing values (tokens embedded in a URL,
/// `http.extraheader`).
pub(super) fn git_read_only(toks: &[String]) -> bool {
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
        // Only the bare name listing: `-v` and `get-url` print URLs, which
        // can embed tokens.
        "remote" => toks.len() == 2,
        _ => true,
    }
}
