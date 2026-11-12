//! `arkavo agent revoke`: the owner ends what this machine's agent may do.
//!
//! Like `authorize`, this is the owner's command and not an agent tool.

use arkavo_agent_auth::{AgentAuthClient, AgentAuthConfig, AgentAuthError};
use arkavo_identity::{DELEGATE_SCOPE, IdentitySession};

const HELP: &str = "End what this machine's agent may do for you

USAGE:
    arkavo agent revoke

Opens Arkavo Creator to approve. The account signed in there must be the one
that authorized this agent. The agent can then get no new tokens. One it
already holds runs out within 15 minutes at most (5 when it was authorized with
`arkavo agent authorize`). To let it act again, run `arkavo agent authorize`.

Identity wants a passkey sign-in from the last hour: if it refuses the
sign-in, sign out of Arkavo Creator, sign in again and retry.

OPTIONS:
    -h, --help          Show this help";

/// What an invocation asks for, decided from its arguments alone.
#[derive(Debug, PartialEq, Eq)]
enum Invocation {
    Help,
    Revoke,
}

/// Decide what to do before Creator is opened or identity is asked anything.
fn plan(args: &[String]) -> Result<Invocation, String> {
    let mut help = false;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" | "help" => help = true,
            other => {
                return Err(format!(
                    "unexpected argument '{other}'\nRun 'arkavo agent revoke --help' for usage"
                ));
            }
        }
    }
    Ok(if help {
        Invocation::Help
    } else {
        Invocation::Revoke
    })
}

/// A revoke failure in words the owner can act on.
fn explain(err: &AgentAuthError) -> String {
    match err {
        AgentAuthError::OwnerSignInRejected(_) => format!(
            "{err}\nRevoking an agent needs a passkey sign-in from the last hour. Sign out of \
             Arkavo Creator, sign in again with your passkey, and run this command again."
        ),
        AgentAuthError::RevokeRefused(_) => format!(
            "{err}\nOnly the account that authorized this agent can revoke it. If that is not \
             the account signed in to Arkavo Creator, sign in there as the right one and retry."
        ),
        _ => err.to_string(),
    }
}

// See `authorize::execute` for why `block_on` is allowed here.
#[allow(clippy::disallowed_methods)]
pub(super) fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if plan(args)? == Invocation::Help {
        println!("{HELP}");
        return Ok(());
    }
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(run()),
        Err(_) => tokio::runtime::Runtime::new()?.block_on(run()),
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Never mint an identity just to revoke it.
    let Some(keypair) = arkavo_registration::existing_agent_keypair()? else {
        println!("This machine's agent has no identity yet, so there is nothing to revoke.");
        return Ok(());
    };
    let agent_did = keypair.public_key().to_did_key();
    println!("Revoking what this machine's agent may do for you.");
    println!("    Agent:   {agent_did}");
    println!("Approve the sign-in in Arkavo Creator, as the account that authorized this agent.\n");

    let owner_token = IdentitySession::new()
        .step_up(DELEGATE_SCOPE)
        .await
        .map_err(|e| super::authorize::explain_sign_in(&e))?;
    println!("    Owner:   {}", arkavo_identity::cwt::sub(&owner_token)?);

    // The owner decides once: a refusal is theirs to act on, not to retry.
    AgentAuthClient::with_config(AgentAuthConfig::default().with_max_retries(0))?
        .revoke(&owner_token, &agent_did)
        .await
        .map_err(|e| explain(&e))?;
    println!(
        "Revoked. The agent can get no new tokens; one it already holds runs out within 15 \
         minutes at most. Run `arkavo agent authorize` to let it act for you again."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn no_arguments_plans_a_revoke() {
        assert_eq!(plan(&[]), Ok(Invocation::Revoke));
    }

    #[test]
    fn help_wins_and_unknown_arguments_are_refused() {
        assert_eq!(plan(&args(&["--help"])), Ok(Invocation::Help));
        let err = plan(&args(&["--all"])).unwrap_err();
        assert!(err.contains("--all"), "{err}");
        assert!(err.contains("arkavo agent revoke --help"), "{err}");
    }

    #[test]
    fn a_stale_sign_in_says_how_to_get_a_fresh_one() {
        let text = explain(&AgentAuthError::OwnerSignInRejected("Invalid token".into()));
        assert!(text.contains("Invalid token"), "{text}");
        assert!(text.contains("sign in again"), "{text}");
    }

    /// Identity's contract says only the owner may revoke and does not pin
    /// the refusal's wording, so whatever it answers is shown, with the one
    /// cause the owner can check for themselves.
    #[test]
    fn a_refusal_keeps_identitys_words_and_names_the_cause_the_owner_can_check() {
        let refusal = "Forbidden";
        let text = explain(&AgentAuthError::RevokeRefused(refusal.into()));
        assert!(text.contains(refusal), "{text}");
        assert!(text.contains("authorized this agent"), "{text}");
    }
}
