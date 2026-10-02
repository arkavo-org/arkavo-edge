//! Names this process holds a credential under by the operator's choice.
//!
//! A provider's `auth_ref` may name any variable (`CORP_LLM_LOGIN`), so no
//! naming convention recognises it, and the name is known only once the
//! configuration naming it is read, long after the code that spawns tool
//! children was written. Whatever reads such a credential registers its
//! name here, and every toolchain environment this process resolves from
//! its own variables withholds it.
//!
//! Names are only ever added: a credential that was read stays in this
//! process's environment, so it stays withheld.

use std::collections::BTreeSet;
use std::sync::{OnceLock, PoisonError, RwLock};

fn registry() -> &'static RwLock<BTreeSet<String>> {
    static NAMES: OnceLock<RwLock<BTreeSet<String>>> = OnceLock::new();
    NAMES.get_or_init(RwLock::default)
}

/// Withhold `name` from every toolchain child this process starts from now
/// on, because a credential was, or is about to be, read from it.
pub fn withhold_name(name: &str) {
    // A panic elsewhere while holding the lock cannot leave a set of
    // strings half-updated, so a poisoned lock is still used: refusing to
    // record the name would let the credential through.
    registry()
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(name.to_owned());
}

/// Every name registered with [`withhold_name`].
pub fn withheld_names() -> Vec<String> {
    registry()
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .cloned()
        .collect()
}
