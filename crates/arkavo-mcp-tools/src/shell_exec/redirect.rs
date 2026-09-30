//! Confinement of shell redirections to the workspace root.

use std::path::Path;

/// Descriptor targets that discard output; writing to them changes nothing.
const DISCARD_TARGETS: &[&str] = &["/dev/null", "NUL", "nul"];

/// A redirection operator with the word it applies to: `span` covers both,
/// in char offsets into the segment.
struct Redirection {
    span: std::ops::Range<usize>,
    target: String,
    duplicates: bool,
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
            span: op..i,
            target: chars[start..i].iter().collect(),
            duplicates,
        });
    }
    found
}

/// Every file redirection in `seg` must name a literal path that resolves
/// inside `root`, symlinks followed. A pure descriptor duplication (`2>&1`,
/// `>&-`) names no file; `>&word` with any other word redirects to that file.
pub(super) fn redirections_within_root(seg: &str, root: &Path, cwd: &Path) -> bool {
    let chars: Vec<char> = seg.chars().collect();
    redirections(&chars).iter().all(|r| {
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

fn literal_target_within_root(target: &str, root: &Path, cwd: &Path) -> bool {
    if DISCARD_TARGETS.contains(&target) {
        return true;
    }
    if target.is_empty() || target.chars().any(shell_expands) {
        return false;
    }
    // A relative target is opened relative to the directory the command runs
    // in, not the root; an absolute one replaces it.
    cwd.join(target)
        .to_str()
        .is_some_and(|path| arkavo_validation::resolve_within_root(root, path).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

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
            ["cat", "/etc/passwd", "2"]
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
}
