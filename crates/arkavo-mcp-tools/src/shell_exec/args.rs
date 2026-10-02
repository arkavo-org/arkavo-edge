//! Argument words as the program receives them, and the rule that keeps an
//! auto-approved command's arguments inside the working tree.
//!
//! These are string rules, not a shell lexer: they refuse spellings that can
//! reach outside the directory the command runs in, and leave symlinks inside
//! the workspace to OS confinement.

/// The words of `seg` as the program receives them, for the bases whose
/// options are checked. Quotes are removed, so `-e''xec` is judged as
/// `-exec` (a backslash never gets here; `classify` refuses it). `None` when
/// a word can still change at run time: a `$` expansion, whose value the
/// caller's env map can choose, or a bash brace expansion such as `-{exec,}`.
pub(super) fn option_words(seg: &str) -> Option<Vec<String>> {
    seg.split_whitespace()
        .map(|word| {
            let braces = word.contains('{') && (word.contains(',') || word.contains(".."));
            (!word.contains('$') && !braces).then(|| word.replace(['\'', '"'], ""))
        })
        .collect()
}

/// Whether `arg` is the long option `name`, or a GNU-style abbreviation of it
/// (getopt_long accepts any unambiguous prefix), with or without `=value`.
pub(super) fn long_option(arg: &str, name: &str) -> bool {
    let given = arg.split('=').next().unwrap_or(arg);
    given.len() > 2 && given.starts_with("--") && name.starts_with(given)
}

/// Whether `arg` is a cluster of short options (`-nro`) containing one of
/// `letters`.
pub(super) fn short_option(arg: &str, letters: &[char]) -> bool {
    arg.len() > 1
        && arg.starts_with('-')
        && !arg.starts_with("--")
        && arg[1..].chars().any(|c| letters.contains(&c))
}

/// Whether every argument word of `seg` (redirections already removed) stays
/// inside the working tree. A word that can still expand fails, as does one
/// that, or whose glued option value, names a path outside.
pub(super) fn arguments_stay_inside(seg: &str) -> bool {
    option_words(seg).is_some_and(|words| !words.iter().any(|w| names_outside(w)))
}

/// Checks the word itself and every place a program may start reading a path
/// inside it: after `=` (`--file=/etc/x`), after `:` (`file -m a:/etc/x`, a
/// search list), and after any letter of a short-option cluster (`-f/etc/x`),
/// since which letter takes the value is the program's choice.
fn names_outside(word: &str) -> bool {
    let cluster = word.starts_with('-') && !word.starts_with("--");
    let mut prev = None;
    word.char_indices().any(|(i, c)| {
        let value_starts = i == 0 || cluster || matches!(prev, Some('=' | ':'));
        prev = Some(c);
        value_starts && path_outside(&word[i..])
    })
}

/// An absolute or home-relative path, a `..` component, or a glob in a
/// dot-led component, which can match `..` (`.*`, `.[.]`). Under `cmd /C` a
/// backslash separates components and a drive letter is absolute.
fn path_outside(path: &str) -> bool {
    let path = if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path.to_string()
    };
    let drive = cfg!(windows) && path.get(1..2) == Some(":");
    drive
        || path.starts_with('/')
        || path.starts_with('~')
        || path.split('/').any(|component| {
            component == ".." || (component.starts_with('.') && component.contains(['*', '?', '[']))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glued_values_are_judged_where_they_start() {
        for word in [
            "--file=/etc/x",
            "-f/etc/x",
            "-nf../x",
            "a:/etc/x",
            "x=~/y",
            "-C~",
        ] {
            assert!(names_outside(word), "{word}");
        }
        for word in [
            "--file=src/x",
            "-d:",
            "-f1",
            "HEAD~1",
            "HEAD:src/main.rs",
            "*.rs",
        ] {
            assert!(!names_outside(word), "{word}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_absolute_and_parent_paths_are_outside() {
        for word in [
            "C:\\Users\\x",
            "c:/x",
            "\\\\server\\share",
            "..\\x",
            "sub\\..\\..\\x",
        ] {
            assert!(names_outside(word), "{word}");
        }
        assert!(!names_outside("src\\lib.rs"));
    }
}
