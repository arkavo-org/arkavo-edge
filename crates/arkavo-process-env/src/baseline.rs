//! Variables every child gets so that ordinary programs start: program
//! lookup, a home and temp directory, identity and locale. None of them
//! carries a credential.

// `pub(crate)` is the real, intended visibility here (the module is private,
// so nothing leaks past the crate either way); `redundant_pub_crate` wants
// `pub`, which `unreachable_pub` then rejects.
#![allow(clippy::redundant_pub_crate)]

use std::ffi::OsStr;

#[cfg(not(windows))]
const NAMES: &[&str] = &[
    "PATH", "HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "TZ", "LANG", "LANGUAGE",
];

// Without `SystemRoot` Winsock and much of the Windows runtime fail to
// initialise; the rest are what installers and runtimes (node, python)
// resolve their directories from.
#[cfg(windows)]
const NAMES: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "USERNAME",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "CommonProgramFiles(x86)",
    "CommonProgramW6432",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
    "ALLUSERSPROFILE",
    "PUBLIC",
    "USERDOMAIN",
    "COMPUTERNAME",
    "PROCESSOR_ARCHITEW6432",
];

/// The POSIX locale categories. `LC_` alone would also admit
/// `LC_<anything>`, which is an operator-invented name, not a locale.
#[cfg(not(windows))]
const LOCALE_CATEGORIES: &[&str] = &[
    "LC_ALL",
    "LC_CTYPE",
    "LC_COLLATE",
    "LC_MESSAGES",
    "LC_MONETARY",
    "LC_NUMERIC",
    "LC_TIME",
    "LC_ADDRESS",
    "LC_IDENTIFICATION",
    "LC_MEASUREMENT",
    "LC_NAME",
    "LC_PAPER",
    "LC_TELEPHONE",
];

pub(crate) fn is_baseline(name: &OsStr) -> bool {
    NAMES
        .iter()
        .any(|known| crate::same_name(name, OsStr::new(known)))
        || is_locale_category(name)
}

#[cfg(not(windows))]
fn is_locale_category(name: &OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| LOCALE_CATEGORIES.contains(&name))
}

#[cfg(windows)]
fn is_locale_category(_name: &OsStr) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_and_locale_are_baseline_credentials_are_not() {
        assert!(is_baseline(OsStr::new("PATH")));
        assert!(!is_baseline(OsStr::new("OPENAI_API_KEY")));
        assert!(!is_baseline(OsStr::new("CARGO_MANIFEST_DIR")));
        #[cfg(not(windows))]
        assert!(is_baseline(OsStr::new("LC_CTYPE")));
    }

    #[cfg(not(windows))]
    #[test]
    fn only_posix_locale_categories_are_baseline() {
        assert!(is_baseline(OsStr::new("LC_ALL")));
        assert!(is_baseline(OsStr::new("LC_TELEPHONE")));
        assert!(!is_baseline(OsStr::new("LC_ACCESS_TOKEN")));
        assert!(!is_baseline(OsStr::new("LC_")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_names_match_whatever_case_the_parent_used() {
        assert!(is_baseline(OsStr::new("Path")));
        assert!(is_baseline(OsStr::new("SYSTEMROOT")));
    }
}
