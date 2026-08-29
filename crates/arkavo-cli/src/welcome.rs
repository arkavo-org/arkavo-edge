//! Welcome display for first-run experience
//!
//! Shows authorization QR code and setup information.

use crate::commands::agent_config::DEFAULT_AGENT_ENTITLEMENTS;
use arkavo_crypto::AgentKeypair;
use arkavo_device_identity::{get_or_create_device_id, keypair};
use arkavo_registration::{AgentDescriptor, qr::display_authorization_qr};

/// Display welcome message with QR code (verbose mode)
pub fn display_welcome_verbose() -> Result<(), Box<dyn std::error::Error>> {
    println!("Welcome Friend\n");

    // Get or create device ID
    let _device_id = get_or_create_device_id()?;

    // Get or create the agent's OWN identity keypair -- distinct from the
    // device keypair. This is the key `agent.rs`'s `--trust` QR authorizes
    // and the one authnz-rs issues a CWT to; the device keypair can never
    // obtain a token, so a QR built from it would leave a human authorizing
    // an identity that will never be used.
    let keypair_bytes = match keypair::get_agent_keypair()? {
        Some(bytes) => bytes,
        None => {
            let new_keypair = AgentKeypair::generate();
            let bytes = new_keypair.to_bytes();
            keypair::store_agent_keypair(&bytes)?;
            bytes
        }
    };

    let agent_keypair = AgentKeypair::from_bytes(&keypair_bytes)?;
    let public_key = agent_keypair.public_key();
    let agent_did = public_key.to_did_key();

    // Get hostname for endpoint
    let hostname = std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "localhost".to_string());

    // Create agent descriptor with DID:key and the agent's requested
    // entitlements (attribute FQNs, the same default set `agent.rs` uses),
    // matching the `--trust` flow so the two QR paths stay consistent.
    let short_id = &public_key.to_base64()[..7.min(public_key.to_base64().len())];
    let descriptor = AgentDescriptor::new(
        public_key,
        format!("{hostname}._a2a._tcp.local."),
        Some(format!("{hostname}._a2a._tcp.local.")),
        short_id.to_string(),
    )
    .with_name(&hostname)
    .with_entitlements(
        DEFAULT_AGENT_ENTITLEMENTS
            .iter()
            .map(|s| s.to_string())
            .collect(),
    );

    // Display authorization QR code with DID:key
    display_authorization_qr(&descriptor)?;

    // Printed so the human can compare it with the DID shown in the phone
    // app before approving the agent's own authnz-rs identity.
    println!("Agent DID: {agent_did}");

    println!();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_device_identity::test_utils::KeypairSlotGuard;
    use std::sync::Mutex;

    // Serializes tests that touch the on-disk keypair slots.
    static KEYCHAIN_MUTEX: Mutex<()> = Mutex::new(());

    /// Regression (R22): the welcome QR must authorize the AGENT keypair,
    /// not the DEVICE keypair. Before this fix, `display_welcome_verbose`
    /// read/wrote the device slot, so a human scanning the welcome QR would
    /// authorize a key that authnz-rs can never issue a CWT to.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn welcome_uses_agent_keypair_not_device_keypair() {
        let _lock = KEYCHAIN_MUTEX.lock().unwrap();
        // Restores the developer's real slots on drop, including after a
        // failing assertion below.
        let _slots = KeypairSlotGuard::capture();

        display_welcome_verbose().expect("welcome flow should succeed");

        assert!(
            keypair::get_agent_keypair().unwrap().is_some(),
            "welcome flow must create/use the agent keypair slot"
        );
        assert!(
            keypair::get_keypair().unwrap().is_none(),
            "welcome flow must not create or touch the device keypair slot"
        );
    }

    /// Regression (R22): entitlements advertised by the welcome QR must come
    /// from `DEFAULT_AGENT_ENTITLEMENTS` (attribute FQNs), not hardcoded
    /// old-vocabulary capability-string literals.
    #[test]
    fn default_agent_entitlements_are_attribute_fqns() {
        assert!(
            DEFAULT_AGENT_ENTITLEMENTS
                .iter()
                .all(|e| e.starts_with("https://arkavo.ai/attr/")),
            "welcome.rs must reuse DEFAULT_AGENT_ENTITLEMENTS, not capability-string literals"
        );
    }
}
