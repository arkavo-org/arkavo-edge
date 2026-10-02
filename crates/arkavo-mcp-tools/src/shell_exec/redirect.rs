//! Confinement of shell redirections to the workspace root.

use std::path::Path;

/// Descriptor targets that discard output; writing to them changes nothing.
/// `NUL` is a device only under `cmd`; on unix it is an ordinary file name,
/// and a symlink of that name must be resolved like any other target.
/// Conversely `/dev/null` is a device only on unix: under `cmd` it names
/// `<drive>:\dev\null`, an ordinary path outside the root.
#[cfg(windows)]
const DISCARD_TARGETS: &[&str] = &["NUL", "nul"];
#[cfg(not(windows))]
const DISCARD_TARGETS: &[&str] = &["/dev/null"];

/// A redirection operator with the word it applies to: `span` covers both
/// and any descriptor number before the operator (`2` in `2>x`), in char
/// offsets into the segment.
struct Redirection {
    span: std::ops::Range<usize>,
    target: String,
    duplicates: bool,
    /// The operator is attached to a preceding word that is not a bare
    /// descriptor number (`-delete>x`).
    glued: bool,
}

/// Every redirection in `seg`. The shell treats an unquoted `<` or `>` as a
/// redirection wherever it appears, glued to a word (`echo hi>out`) or not,
/// so the scan is character-level and quote-aware rather than per whitespace
/// token.
fn redirections(chars: &[char]) -> Vec<Redirection> {
    let mut found = Vec::new();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if is_quote(c) {
            quote = Some(c);
            i += 1;
            continue;
        }
        if c != '<' && c != '>' {
            i += 1;
            continue;
        }
        let op = i;
        // A descriptor number is part of the redirection; any other word
        // touching the operator is not separable from it by whitespace, so
        // the argument checks that compare whole words cannot see it.
        let mut first = op;
        while first > 0 && chars[first - 1].is_ascii_digit() {
            first -= 1;
        }
        // `cmd` takes a single digit as the descriptor; a longer run is not
        // one, so it is refused as a glued word.
        let glued =
            (first > 0 && !chars[first - 1].is_whitespace()) || (cfg!(windows) && op - first > 1);
        while i < chars.len() && matches!(chars[i], '<' | '>') {
            i += 1;
        }
        let duplicates = chars.get(i) == Some(&'&');
        if duplicates {
            i += 1;
        }
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && !matches!(chars[i], '<' | '>') {
            i += 1;
        }
        found.push(Redirection {
            span: first..i,
            target: chars[start..i].iter().collect(),
            duplicates,
            glued,
        });
    }
    found
}

/// Every file redirection in `seg` must name a literal path that resolves
/// inside `root`, symlinks followed, and stand apart from the word before it
/// (`echo hi>out` and `find . -delete>x` are refused; `2>out` is not). A pure
/// descriptor duplication (`2>&1`, `>&-`) names no file; `>&word` with any
/// other word redirects to that file.
pub(super) fn redirections_within_root(seg: &str, root: &Path, cwd: &Path) -> bool {
    let chars: Vec<char> = seg.chars().collect();
    redirections(&chars).iter().all(|r| {
        if r.glued {
            return false;
        }
        let names_descriptor = r.target == "-"
            || (!r.target.is_empty() && r.target.chars().all(|d| d.is_ascii_digit()));
        (r.duplicates && names_descriptor) || literal_target_within_root(&r.target, root, cwd)
    })
}

/// `seg` with each redirection and its target blanked out, leaving the words
/// the program receives as arguments. Targets are judged above, against the
/// root, so the argument rules must not see them.
pub(super) fn without_redirections(seg: &str) -> String {
    let mut chars: Vec<char> = seg.chars().collect();
    for r in redirections(&chars) {
        chars[r.span].fill(' ');
    }
    chars.into_iter().collect()
}

/// `cmd /C` has no single-quote quoting: `echo it's > C:\x` really redirects,
/// so treating `'` as opening a quote there would hide the target.
fn is_quote(c: char) -> bool {
    c == '"' || (c == '\'' && cfg!(not(windows)))
}

/// Characters the shell expands or unquotes in a word. A target containing
/// one cannot be judged from the string: `$HOME/.bashrc` and `~/.bashrc`
/// would otherwise resolve as literal directories inside the root. `%`, `!`
/// and `^` do the same under `cmd /C`, where `\` is a path separator rather
/// than an escape.
fn shell_expands(c: char) -> bool {
    matches!(
        c,
        '$' | '~' | '*' | '?' | '[' | ']' | '{' | '}' | '\'' | '"' | '`' | '%' | '!' | '^'
    ) || (c == '\\' && cfg!(not(windows)))
}

/// Names `cmd` opens as devices in any directory, with or without an
/// extension (`aux.txt`), so a redirection to one never creates a file.
fn is_reserved_device(target: &str) -> bool {
    let name = target.rsplit(['/', '\\']).next().unwrap_or(target);
    let stem = name
        .split(['.', ':', ' '])
        .next()
        .unwrap_or(name)
        .to_ascii_uppercase();
    match stem.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" => true,
        _ => {
            let numbered = stem
                .strip_prefix("COM")
                .or_else(|| stem.strip_prefix("LPT"));
            // Windows also reserves COM0/LPT0 and the superscript digits in
            // some versions; refusing them all costs nothing.
            numbered.is_some_and(|n| {
                let mut digits = n.chars();
                matches!(
                    (digits.next(), digits.next()),
                    (Some('0'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'), None)
                )
            })
        }
    }
}

fn literal_target_within_root(target: &str, root: &Path, cwd: &Path) -> bool {
    if DISCARD_TARGETS.contains(&target) {
        return true;
    }
    if target.is_empty()
        || target.chars().any(shell_expands)
        || (cfg!(windows) && is_reserved_device(target))
    {
        return false;
    }
    // A relative target is opened relative to the directory the command runs
    // in, not the root; an absolute one replaces it.
    cwd.join(target)
        .to_str()
        .and_then(|path| arkavo_validation::resolve_within_root(root, path).ok())
        .is_some_and(|resolved| !names_git_control(&resolved))
}

/// Git reads these and runs what they configure: a write to `.git/config`
/// (`core.fsmonitor`, `diff.external`) turns the next auto-approved
/// `git status` or `git diff` into a launcher. Judged on the resolved path so
/// a symlink into `.git` counts, and case-insensitively because macOS and
/// Windows file systems are.
fn names_git_control(resolved: &Path) -> bool {
    resolved.components().any(|c| {
        let name = c.as_os_str();
        [".git", ".gitattributes", ".gitmodules"]
            .iter()
            .any(|control| name.eq_ignore_ascii_case(control))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: `echo 'fsmonitor = ./x.sh' >> .git/config` was auto-approved,
    // and a following auto-approved `git status` then ran ./x.sh.
    #[test]
    fn git_control_files_are_not_auto_approved_targets() {
        let (_dir, root) = workspace();
        std::fs::create_dir(root.join(".git")).unwrap();
        for seg in [
            "echo x >> .git/config",
            "echo x > ./.git/hooks/pre-commit",
            "echo x >> .GIT/config",
            "echo x > .gitattributes",
            "echo x > sub/.gitmodules",
        ] {
            assert!(!redirections_within_root(seg, &root, &root), "{seg}");
        }
        assert!(redirections_within_root(
            "echo x > .gitignore",
            &root,
            &root
        ));
        assert!(redirections_within_root("echo x > notes.git", &root, &root));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_into_git_dir_is_not_an_auto_approved_target() {
        let (_dir, root) = workspace();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::os::unix::fs::symlink(root.join(".git"), root.join("g")).unwrap();
        assert!(!redirections_within_root(
            "echo x >> g/config",
            &root,
            &root
        ));
    }

    fn workspace() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        (dir, root)
    }

    #[test]
    fn quoted_angle_brackets_are_not_redirections() {
        let (_dir, root) = workspace();
        assert!(redirections_within_root(
            "echo \"a > /etc/x\"",
            &root,
            &root
        ));
        assert!(redirections_within_root("grep \"<b>\" f", &root, &root));
        assert!(redirections_within_root("ls", &root, &root));
    }

    #[test]
    fn descriptor_duplication_names_no_file() {
        let (_dir, root) = workspace();
        for seg in ["ls 2>&1", "ls >&2", "ls >&-", "ls 2>&-", "cat <&0"] {
            assert!(redirections_within_root(seg, &root, &root), "{seg}");
        }
        // Any other word after `>&` is a file.
        assert!(redirections_within_root("ls >&out.txt", &root, &root));
        assert!(!redirections_within_root("ls >&/etc/passwd", &root, &root));
        assert!(!redirections_within_root("ls >& /etc/passwd", &root, &root));
    }

    #[test]
    fn every_redirection_in_a_segment_is_checked() {
        let (_dir, root) = workspace();
        assert!(redirections_within_root(
            "ls > a.txt 2> b.txt",
            &root,
            &root
        ));
        assert!(!redirections_within_root(
            "ls > a.txt 2> /etc/b",
            &root,
            &root
        ));
        assert!(redirections_within_root("ls >> a.txt", &root, &root));
        assert!(!redirections_within_root("ls >>/etc/a", &root, &root));
        assert!(!redirections_within_root("ls &>/etc/a", &root, &root));
        assert!(!redirections_within_root("cat <</etc/a", &root, &root));
        assert!(!redirections_within_root("ls 2>&1 >/etc/a", &root, &root));
    }

    #[test]
    fn quoted_or_expanded_targets_are_not_judged_literal() {
        let (_dir, root) = workspace();
        for target in [
            "\"out.txt\"",
            "$X",
            "~",
            "*.txt",
            "{a,b}",
            "`x`",
            "a?b",
            "!x",
        ] {
            let seg = format!("ls > {target}");
            assert!(!redirections_within_root(&seg, &root, &root), "{seg}");
        }
    }

    #[test]
    fn stripping_leaves_only_argument_words() {
        assert_eq!(
            without_redirections("cat /etc/passwd>out.txt 2>&1")
                .split_whitespace()
                .collect::<Vec<_>>(),
            ["cat", "/etc/passwd"]
        );
        assert_eq!(
            without_redirections("echo \"a > b\" > /abs/in/root")
                .split_whitespace()
                .collect::<Vec<_>>(),
            ["echo", "\"a", ">", "b\""]
        );
        assert_eq!(without_redirections("ls"), "ls");
    }

    #[test]
    fn discard_targets_stay_approvable() {
        let (_dir, root) = workspace();
        assert!(redirections_within_root(
            "ls > /dev/null 2>&1",
            &root,
            &root
        ));
        assert!(redirections_within_root("ls 2>/dev/null", &root, &root));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_file_target_is_judged_by_where_it_points() {
        use std::os::unix::fs::symlink;
        let (_dir, root) = workspace();
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("victim");
        std::fs::write(&victim, "x").unwrap();
        symlink(&victim, root.join("alias")).unwrap();
        std::fs::write(root.join("real"), "x").unwrap();
        symlink(root.join("real"), root.join("inner")).unwrap();
        assert!(!redirections_within_root("ls > alias", &root, &root));
        assert!(redirections_within_root("ls > inner", &root, &root));
    }

    #[cfg(windows)]
    #[test]
    fn single_quote_does_not_hide_a_redirection_under_cmd() {
        let (_dir, root) = workspace();
        assert!(!redirections_within_root(
            "echo it's > C:\\Windows\\x",
            &root,
            &root
        ));
    }

    #[test]
    fn a_word_glued_to_an_operator_is_refused_unless_it_is_a_descriptor() {
        let (_dir, root) = workspace();
        for seg in [
            "echo hi>out",
            "x>y",
            "find . -delete>x",
            "echo a2>x",
            "ls<x",
        ] {
            assert!(!redirections_within_root(seg, &root, &root), "{seg}");
        }
        for seg in ["ls 2>x", "ls 1>>x", "ls 2>/dev/null", "ls 0<x", "> x ls"] {
            assert!(redirections_within_root(seg, &root, &root), "{seg}");
        }
    }

    #[test]
    fn descriptor_prefix_is_removed_with_its_redirection() {
        let words = |seg: &str| {
            without_redirections(seg)
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        assert_eq!(words("uniq f 2>&1"), ["uniq", "f"]);
        assert_eq!(words("ls 2> out.txt -l"), ["ls", "-l"]);
        assert_eq!(words("ls 12>x"), ["ls"]);
        // A number that is a word of its own stays an argument.
        assert_eq!(words("head -n 5 f 2>x"), ["head", "-n", "5", "f"]);
    }

    #[cfg(unix)]
    #[test]
    fn nul_is_an_ordinary_file_name_on_unix() {
        use std::os::unix::fs::symlink;
        let (_dir, root) = workspace();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path().join("victim"), root.join("NUL")).unwrap();
        assert!(!redirections_within_root("ls > NUL", &root, &root));
        assert!(redirections_within_root("ls > /dev/null", &root, &root));
    }

    #[test]
    fn reserved_device_names_are_recognised_with_or_without_extension() {
        for name in [
            "CON",
            "con",
            "PRN",
            "AUX",
            "COM1",
            "com9",
            "LPT1",
            "lpt9",
            "CON.txt",
            "aux.log",
            "sub/COM3",
            "sub\\LPT2.x",
            "CONIN$",
            "CONOUT$",
            "COM0",
            "lpt0.txt",
            "COM\u{b9}",
            "COM\u{b2}",
            "COM\u{b3}",
            "LPT\u{b9}.log",
            "LPT\u{b2}",
            "LPT\u{b3}",
        ] {
            assert!(is_reserved_device(name), "{name}");
        }
        for name in ["console", "COM10", "LPT", "out.txt", "a/b", "NUL2"] {
            assert!(!is_reserved_device(name), "{name}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn only_a_single_digit_is_a_descriptor_prefix_under_cmd() {
        let (_dir, root) = workspace();
        assert!(redirections_within_root("dir 2>x", &root, &root));
        assert!(!redirections_within_root("dir 12>x", &root, &root));
        assert!(!redirections_within_root("12>x dir", &root, &root));
    }

    #[cfg(windows)]
    #[test]
    fn dev_null_is_an_ordinary_path_under_cmd() {
        let (_dir, root) = workspace();
        assert!(!redirections_within_root("dir > /dev/null", &root, &root));
    }

    #[cfg(windows)]
    #[test]
    fn device_targets_are_refused_but_nul_discards_under_cmd() {
        let (_dir, root) = workspace();
        assert!(redirections_within_root("dir > NUL", &root, &root));
        assert!(redirections_within_root("dir > nul", &root, &root));
        for seg in [
            "dir > CON",
            "dir > prn.txt",
            "dir > COM1",
            "dir > sub\\LPT1",
        ] {
            assert!(!redirections_within_root(seg, &root, &root), "{seg}");
        }
    }
}
