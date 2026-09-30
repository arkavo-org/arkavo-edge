//! Refusing an environment entry that would hand a child a credential or a
//! way to run other code.
//!
//! Three places accept an operator-declared environment (a kit's MCP server,
//! the proxy's `ProxyConfig`, the proxy's `--env` flags) and each must refuse
//! the same entries. The refusal carries the name and never the value: a
//! value refused here may be the credential itself.

use crate::{EnvSpec, hijack::is_loader_or_hijack_name, is_secret_name, is_valid_name};

/// Why an environment entry was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvRefusal {
    /// The name is empty or contains `=` or NUL. It is not carried, because
    /// a malformed `NAME=VALUE` split can leave a value in the name.
    InvalidName,
    /// The named variable makes a program load or run other code.
    LoaderName(String),
    /// The named credential-shaped variable was given as a literal, which
    /// would put the credential in configuration or argv.
    CredentialLiteral(String),
}

/// Screens one entry. `literal` is whether the entry sets a value rather
/// than copying the parent's: a credential may only be passed through.
pub fn screen_entry(name: &str, literal: bool) -> Result<(), EnvRefusal> {
    if !is_valid_name(name) {
        return Err(EnvRefusal::InvalidName);
    }
    if is_loader_or_hijack_name(name) {
        return Err(EnvRefusal::LoaderName(name.to_string()));
    }
    if literal && is_secret_name(name) {
        return Err(EnvRefusal::CredentialLiteral(name.to_string()));
    }
    Ok(())
}

impl EnvSpec {
    /// Screens every literal in `set` and every name in `passthrough`,
    /// returning the first refusal.
    pub fn screen(&self) -> Result<(), EnvRefusal> {
        for name in &self.passthrough {
            screen_entry(name, false)?;
        }
        for name in self.set.keys() {
            screen_entry(name, true)?;
        }
        Ok(())
    }
}
