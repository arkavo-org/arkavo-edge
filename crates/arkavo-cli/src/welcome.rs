//! Welcome display for first-run experience
//!
//! Shows authorization QR code and setup information.

use arkavo_crypto::AgentKeypair;
use arkavo_device_identity::{get_or_create_device_id, keypair};
use arkavo_registration::{qr::display_authorization_qr, AgentDescriptor};

/// Display welcome message with QR code (verbose mode)
pub fn display_welcome_verbose() -> Result<(), Box<dyn std::error::Error>> {
    println!("Welcome Friend\n");

    // Get or create device ID
    let _device_id = get_or_create_device_id()?;

    // Get or create keypair
    let keypair_bytes = match keypair::get_keypair()? {
        Some(bytes) => bytes,
        None => {
            let new_keypair = AgentKeypair::generate();
            let bytes = new_keypair.to_bytes();
            keypair::store_keypair(&bytes)?;
            bytes
        }
    };

    let agent_keypair = AgentKeypair::from_bytes(&keypair_bytes)?;
    let public_key = agent_keypair.public_key();

    // Get hostname for endpoint
    let hostname = std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "localhost".to_string());

    // Create agent descriptor with DID:key and default entitlements
    let short_id = &public_key.to_base64()[..7.min(public_key.to_base64().len())];
    let descriptor = AgentDescriptor::new(
        public_key,
        format!("{hostname}._a2a._tcp.local."),
        Some(format!("{hostname}._a2a._tcp.local.")),
        short_id.to_string(),
    )
    .with_name(&hostname)
    .with_entitlements(vec![
        "agent.capability.chat".to_string(),
        "agent.capability.tools".to_string(),
    ]);

    // Display authorization QR code with DID:key
    display_authorization_qr(&descriptor)?;

    println!();

    Ok(())
}
