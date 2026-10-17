//! Taking kit roles from an orchestrator (SwarmKit draft-02 §7): how an
//! agent started without a kit is set up to take them, and
//! `arkavo agent release`.

use std::error::Error;
use std::sync::Arc;

use arkavo_server::server::swarmkit::{ModelResolver, SpecialistConfig};
use arkavo_swarmkit_delegation::StateStore;

/// Check `--orchestrator`'s value before anything starts.
pub(super) fn parse_orchestrator(value: &str) -> Result<String, String> {
    arkavo_crypto::AgentPublicKey::from_did_key(value)
        .map(|_| value.to_string())
        .map_err(|_| {
            format!(
                "--orchestrator takes the Ed25519 did:key `arkavo kit apply` prints, not {value:?}"
            )
        })
}

/// How this agent takes roles. One started with a kit role runs that role
/// and takes no other; one started without a kit binds to the operator's
/// orchestrator, or to the first that assigns it a role.
pub(super) fn specialist_config(
    role_id: Option<&str>,
    orchestrator: Option<&str>,
) -> Result<SpecialistConfig, Box<dyn Error>> {
    let enlistable = role_id.is_none();
    let key = arkavo_registration::load_or_create_agent_keypair()?
        .public_key()
        .to_did_key();
    let resolve_model: ModelResolver = Arc::new(crate::commands::kit::kit_model_to_hint);
    Ok(SpecialistConfig {
        key,
        state_dir: arkavo_device_identity::keypair::identity_dir()?,
        enlistable,
        operator_orchestrator: orchestrator.map(str::to_string),
        resolve_model,
    })
}

/// `arkavo agent release`: forget the orchestrator and the role it gave.
/// A binding that cannot be read is removed too; that is the way out the
/// agent names when it refuses to start taking roles with one.
pub(super) fn release() -> Result<(), Box<dyn Error>> {
    let store = StateStore::new(arkavo_device_identity::keypair::identity_dir()?);
    let binding = store.load_binding();
    store.save_delegation(None)?;
    store.save_binding(None)?;
    match binding {
        Ok(Some(binding)) => println!("Released orchestrator {}.", binding.orchestrator),
        Ok(None) => println!("No orchestrator was bound."),
        Err(e) => println!("Removed an unreadable binding: {e}"),
    }
    println!(
        "An agent that is running keeps its role until it restarts. Without a kit, the next \
         orchestrator to assign it a role becomes its orchestrator, unless it is started with \
         --orchestrator <did:key>."
    );
    Ok(())
}
