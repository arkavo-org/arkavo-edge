//! Which of an environment's variables hold a credential, for a caller that
//! cannot resolve a [`crate::ChildEnv`] and must override names instead.
//!
//! A launcher whose API can set variables on the child but not remove the
//! inherited ones (chromiumoxide) blanks the credentials it names. Those
//! names must be decided by the same policy as every other child, or the
//! launcher is a hole in it.

use crate::{is_secret_name, same_name, withheld_names};
use std::ffi::OsStr;

/// The credential-like names of one environment.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CredentialNames {
    /// Credential names in the parent's own spelling, which a launcher can
    /// override by name.
    pub blankable: Vec<String>,
    /// Credential-like names that are not valid Unicode, in lossy form so
    /// they can be reported. A `String`-keyed launcher cannot override
    /// them, so the caller must refuse to start the child. Never a value.
    pub unblankable: Vec<String>,
}

/// Classifies `names` as credentials or not.
///
/// A credential is credential-shaped (`is_secret_name`) or registered with
/// [`crate::withhold_name`], compared with the crate's `same_name` rules. A name that is not valid Unicode is judged by its lossy form,
/// which keeps the ASCII a credential convention is made of.
pub fn credential_names<N: AsRef<OsStr>>(names: impl IntoIterator<Item = N>) -> CredentialNames {
    let registered = withheld_names();
    let mut found = CredentialNames::default();
    for name in names {
        let name = name.as_ref();
        let lossy = name.to_string_lossy();
        let credential = is_secret_name(&lossy)
            || registered
                .iter()
                .any(|known| same_name(name, OsStr::new(known)));
        if !credential {
            continue;
        }
        match name.to_str() {
            Some(name) => found.blankable.push(name.to_owned()),
            None => found.unblankable.push(lossy.into_owned()),
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::withhold_name;
    use arkavo_test_macros::spec;

    #[spec("MCP-016")]
    #[test]
    fn credential_shaped_and_registered_names_are_found_in_their_own_spelling() {
        // The registry is process-global and tests run in parallel: this
        // name is used by no other test.
        withhold_name("PENV_CREDNAMES_TEST_LOGIN");
        let found = credential_names([
            "Openai_Api_Key",
            "PENV_CREDNAMES_TEST_LOGIN",
            "RUSTFLAGS",
            "HOME",
        ]);
        assert_eq!(
            found.blankable,
            vec!["Openai_Api_Key", "PENV_CREDNAMES_TEST_LOGIN"]
        );
        assert!(found.unblankable.is_empty());
    }

    #[cfg(windows)]
    #[spec("MCP-016")]
    #[test]
    fn registered_names_match_case_insensitively_on_windows() {
        withhold_name("PENV_CREDNAMES_WIN_LOGIN");
        let found = credential_names(["penv_crednames_win_login"]);
        assert_eq!(found.blankable, vec!["penv_crednames_win_login"]);
    }

    #[cfg(unix)]
    #[spec("MCP-016")]
    #[test]
    fn a_non_unicode_credential_name_is_reported_not_blanked() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let credential = OsString::from_vec(b"MY_\xff_TOKEN".to_vec());
        let plain = OsString::from_vec(b"PLAIN_\xff_NAME".to_vec());
        let found = credential_names([credential, plain]);
        assert!(found.blankable.is_empty());
        assert_eq!(found.unblankable, vec!["MY_\u{fffd}_TOKEN"]);
    }
}
