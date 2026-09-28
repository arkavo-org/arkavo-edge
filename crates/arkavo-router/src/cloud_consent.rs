//! Who may answer "yes" to paid cloud inference, and on whose behalf.
//!
//! The router refuses automatic cloud selection under `AskBeforeCloud` and has
//! no channel of its own to reach a person. The host that does own such a
//! channel — a terminal, a UI, an approval webhook — supplies a
//! [`CloudConsentPrompt`]; a host with no channel supplies none and the refusal
//! propagates unchanged. Nothing in this module reads stdin, so a library
//! embedded in a server can never block a remote request on the server's
//! console.
//!
//! Approvals are recorded per session in a [`CloudConsentLedger`], never as one
//! process-wide flag: the answer one user gives for their own session must not
//! authorize the next session the same process serves.

use std::collections::HashSet;
use std::sync::RwLock;

/// What a host is being asked to approve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CloudConsentRequest<'a> {
    /// One routing call was refused: this arm, at this estimated cost.
    Call {
        model: &'a str,
        estimated_cost_usd: f64,
    },
    /// Asked up front, before any work: cloud arms are configured and the
    /// policy needs an answer before they can be drawn at all.
    Session,
}

/// The host's channel to the person who decides whether to spend.
///
/// Implementations live in the host binary, never in a library crate. `ask`
/// returns the answer; anything other than an explicit approval is a decline,
/// and a host that cannot ask right now answers `false` rather than blocking.
#[async_trait::async_trait]
pub trait CloudConsentPrompt: Send + Sync {
    async fn ask(&self, request: CloudConsentRequest<'_>) -> bool;
}

/// Whose approval a routing call may draw on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ConsentKey {
    /// A named user session. Chat sessions always name their own id, so an
    /// approval recorded here reaches exactly one conversation.
    Session(String),
    /// Work the host process does on its own behalf — an agent's conductor
    /// calls, intent analysis, internal synthesis. These carry no session id
    /// because no user session owns them; the operator at the process's own
    /// terminal is the only one who can approve this key, and because every
    /// user session names itself, that approval can never reach one.
    Host,
}

impl ConsentKey {
    fn of(session: Option<&str>) -> Self {
        match session {
            Some(id) => Self::Session(id.to_string()),
            None => Self::Host,
        }
    }
}

/// Standing cloud approvals, keyed by who gave them.
///
/// An entry is never removed and never consumed: a command that approves cloud
/// once fans out into many routing calls it does not issue itself, and a
/// one-shot flag would be spent by the first of them. A decline is not recorded
/// at all — the caller that asked remembers it, which keeps "asked and refused"
/// as final as "asked and approved" without the ledger growing a second state.
#[derive(Debug, Default)]
pub struct CloudConsentLedger {
    approved: RwLock<HashSet<ConsentKey>>,
}

impl CloudConsentLedger {
    /// Record an approval for one user session.
    pub fn approve_session(&self, session_id: &str) {
        self.insert(ConsentKey::Session(session_id.to_string()));
    }

    /// Record an approval for the host process's own work.
    pub fn approve_host(&self) {
        self.insert(ConsentKey::Host);
    }

    /// Whether the owner of this call has approved cloud spend. `None` asks
    /// about the host's own work, not "anyone at all".
    pub fn is_approved(&self, session: Option<&str>) -> bool {
        self.read().contains(&ConsentKey::of(session))
    }

    fn insert(&self, key: ConsentKey) {
        // Recover on poison: the set is a plain collection of approvals, a
        // panic elsewhere cannot leave it half-written, and losing every
        // standing approval would silently re-ask the user.
        match self.approved.write() {
            Ok(mut set) => {
                set.insert(key);
            }
            Err(poisoned) => {
                poisoned.into_inner().insert(key);
            }
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, HashSet<ConsentKey>> {
        match self.approved.read() {
            Ok(set) => set,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("ASTRA-004")]
    #[test]
    fn an_approval_reaches_only_the_session_that_gave_it() {
        let ledger = CloudConsentLedger::default();
        ledger.approve_session("session-a");

        assert!(ledger.is_approved(Some("session-a")));
        assert!(!ledger.is_approved(Some("session-b")));
        assert!(!ledger.is_approved(None));
    }

    /// The operator at the process's terminal approves the host's own work.
    /// Every user session names itself, so that approval cannot reach one.
    #[spec("ASTRA-004")]
    #[test]
    fn a_host_approval_does_not_authorize_any_session() {
        let ledger = CloudConsentLedger::default();
        ledger.approve_host();

        assert!(ledger.is_approved(None));
        assert!(!ledger.is_approved(Some("session-a")));
    }

    #[spec("ASTRA-004")]
    #[test]
    fn nothing_is_approved_until_someone_approves_it() {
        let ledger = CloudConsentLedger::default();
        assert!(!ledger.is_approved(None));
        assert!(!ledger.is_approved(Some("session-a")));
    }

    /// The approval is standing, not one-shot: the calls a single approval
    /// fans out into must all see it.
    #[spec("ASTRA-004")]
    #[test]
    fn an_approval_is_not_consumed_by_reading_it() {
        let ledger = CloudConsentLedger::default();
        ledger.approve_session("session-a");
        for _ in 0..3 {
            assert!(ledger.is_approved(Some("session-a")));
        }
    }
}
