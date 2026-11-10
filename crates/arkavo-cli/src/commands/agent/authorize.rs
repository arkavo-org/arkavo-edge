//! `arkavo agent authorize`: the owner lets this machine's agent act for them.
//!
//! This is the owner's command, not the agent's. It is not a tool and nothing
//! in the agent loop reaches it: an agent that could start its own
//! authorization would be granting itself what the owner is meant to grant.

use arkavo_agent_auth::{AgentAuthClient, AgentAuthConfig, AgentAuthError, Delegation};
use arkavo_identity::{DELEGATE_SCOPE, IdentityError, IdentitySession};

const HELP: &str = "Let this machine's agent act for you

USAGE:
    arkavo agent authorize --market-requests

Opens Arkavo Creator to approve. Whoever is signed in there becomes the
agent's owner, and the agent may act for them for the next 12 hours; run the
command again to renew. Authorizing replaces whatever the agent was allowed
before.

Identity wants a passkey sign-in from the last hour: if it refuses the
sign-in, sign out of Arkavo Creator, sign in again and retry.

OPTIONS:
    --market-requests   The agent may ask the Arkavo market for purchases,
                        which you then pay (see `arkavo market --help`)
    -h, --help          Show this help";

/// What the owner lets the agent do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grant {
    MarketRequests,
}

impl Grant {
    fn in_words(self) -> &'static str {
        match self {
            Self::MarketRequests => "ask the Arkavo market for purchases, which you then pay",
        }
    }
}

/// What an invocation asks for, decided from its arguments alone.
#[derive(Debug, PartialEq, Eq)]
enum Invocation {
    Help,
    Authorize(Grant),
}

fn refuse(message: &str) -> String {
    format!("{message}\nRun 'arkavo agent authorize --help' for usage")
}

/// Decide what to do before Creator is opened or identity is asked anything.
fn plan(args: &[String]) -> Result<Invocation, String> {
    let mut grant = None;
    let mut help = false;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" | "help" => help = true,
            "--market-requests" => grant = Some(Grant::MarketRequests),
            other => return Err(refuse(&format!("unexpected argument '{other}'"))),
        }
    }
    if help {
        return Ok(Invocation::Help);
    }
    grant.map(Invocation::Authorize).ok_or_else(|| {
        refuse("say what the agent may do: 'arkavo agent authorize --market-requests'")
    })
}

/// The delegation a grant asks identity for.
fn delegation(agent_did: &str, grant: Grant) -> Delegation {
    match grant {
        Grant::MarketRequests => Delegation {
            agent_did: agent_did.to_string(),
            name: "Arkavo Edge".to_string(),
            entitlements: vec![arkavo_market::REQUEST_ENTITLEMENT.to_string()],
            // A market request mints its token right before it is sent, so
            // nothing needs one that lives longer.
            short_lived: true,
        },
    }
}

/// An authorize failure in words the owner can act on.
fn explain(err: &AgentAuthError) -> String {
    match err {
        AgentAuthError::OwnerSignInRejected(_) => format!(
            "{err}\nAuthorizing an agent needs a passkey sign-in from the last hour. Sign out \
             of Arkavo Creator, sign in again with your passkey, and run this command again."
        ),
        _ => err.to_string(),
    }
}

/// A failed Creator sign-in in words the owner can act on.
fn explain_sign_in(err: &IdentityError) -> String {
    match err {
        IdentityError::AccessDenied => "Arkavo Creator answered that the sign-in was declined. \
             It answers the same way when identity refuses the sign-in it forwards, so if you \
             approved it, sign out of Arkavo Creator, sign in again and retry."
            .to_string(),
        _ => err.to_string(),
    }
}

// Both `Handle::block_on` and `Runtime::block_on` are disallowed by
// `.clippy.toml` (nesting a runtime inside a runtime can panic). This is a
// CLI entry point: it reuses an ambient runtime when one exists and only
// builds its own otherwise.
#[allow(clippy::disallowed_methods)]
pub(super) fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let grant = match plan(args)? {
        Invocation::Help => {
            println!("{HELP}");
            return Ok(());
        }
        Invocation::Authorize(grant) => grant,
    };
    let run_async = run(grant);
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(run_async),
        Err(_) => tokio::runtime::Runtime::new()?.block_on(run_async),
    }
}

async fn run(grant: Grant) -> Result<(), Box<dyn std::error::Error>> {
    let agent_did = arkavo_registration::load_or_create_agent_keypair()?
        .public_key()
        .to_did_key();
    println!("Authorizing this machine's agent to act for you.");
    println!("    Agent:   {agent_did}");
    println!("    It may:  {}", grant.in_words());
    println!("This replaces anything the agent was allowed before.\n");
    println!(
        "Approve the sign-in in Arkavo Creator. Whoever is signed in there becomes the agent's \
         owner."
    );

    let owner_token = IdentitySession::new()
        .step_up(DELEGATE_SCOPE)
        .await
        .map_err(|e| explain_sign_in(&e))?;
    println!("    Owner:   {}", arkavo_identity::cwt::sub(&owner_token)?);

    // The owner decides once: a refusal is theirs to act on, not to retry.
    let identity = AgentAuthClient::with_config(AgentAuthConfig::default().with_max_retries(0))?;
    let authorized = identity
        .authorize(&owner_token, &delegation(&agent_did, grant))
        .await
        .map_err(|e| explain(&e))?;
    println!(
        "Authorized until {}. Run this command again to renew.",
        authorized
            .appraised_until
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M %:z")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn market_requests_is_planned_as_that_grant() {
        assert_eq!(
            plan(&args(&["--market-requests"])),
            Ok(Invocation::Authorize(Grant::MarketRequests))
        );
    }

    /// Authorizing replaces what the agent was allowed, so the command never
    /// guesses a grant: without one it explains and contacts nobody.
    #[test]
    fn nothing_is_authorized_without_naming_what() {
        let err = plan(&[]).unwrap_err();
        assert!(err.contains("--market-requests"), "{err}");
    }

    #[test]
    fn help_wins_and_unknown_arguments_are_refused() {
        assert_eq!(plan(&args(&["--help"])), Ok(Invocation::Help));
        assert_eq!(
            plan(&args(&["--market-requests", "-h"])),
            Ok(Invocation::Help)
        );
        let err = plan(&args(&["--market-requests", "--all"])).unwrap_err();
        assert!(err.contains("--all"), "{err}");
    }

    /// Identity refuses the whole call when the owner lacks any entitlement
    /// asked for, and every account holds the market's. So it is asked for
    /// alone, on tokens that live five minutes.
    #[test]
    fn the_market_grant_asks_for_the_markets_entitlement_alone() {
        let delegation = delegation(AGENT, Grant::MarketRequests);

        assert_eq!(delegation.agent_did, AGENT);
        assert_eq!(
            delegation.entitlements,
            [arkavo_market::REQUEST_ENTITLEMENT]
        );
        assert!(delegation.short_lived);
        assert!((1..=64).contains(&delegation.name.len()));
    }

    #[test]
    fn a_stale_sign_in_says_how_to_get_a_fresh_one() {
        let text = explain(&AgentAuthError::OwnerSignInRejected("Unauthorized".into()));
        assert!(text.contains("Unauthorized"), "{text}");
        assert!(text.contains("sign in again"), "{text}");
    }

    /// Creator answers `access_denied` both when the person declines and when
    /// identity refuses it, so a decline is not stated as fact.
    #[test]
    fn a_declined_sign_in_names_both_causes() {
        let text = explain_sign_in(&IdentityError::AccessDenied);
        assert!(text.contains("declined"), "{text}");
        assert!(text.contains("identity"), "{text}");
    }

    #[test]
    fn other_sign_in_failures_are_reported_as_they_are() {
        let text = explain_sign_in(&IdentityError::TimedOut);
        assert_eq!(text, IdentityError::TimedOut.to_string());
    }

    #[test]
    fn a_refusal_keeps_identitys_words() {
        let refusal = "Workload quarantined";
        let text = explain(&AgentAuthError::AuthorizeRefused(refusal.into()));
        assert!(text.contains(refusal), "{text}");
        assert!(!text.contains("sign in again"), "{text}");
    }
}
