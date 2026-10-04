//! Welcome display for first-run experience
//!
//! Shows authorization QR code and setup information.

use arkavo_device_identity::get_or_create_device_id;
use arkavo_registration::{
    AgentDescriptor, default_entitlements, load_or_create_agent_keypair,
    qr::display_authorization_qr,
};

/// Display welcome message with QR code (verbose mode)
pub fn display_welcome_verbose() -> Result<(), Box<dyn std::error::Error>> {
    println!("Welcome Friend\n");

    // Get or create device ID
    let _device_id = get_or_create_device_id()?;

    // The same identity `arkavo agent run --trust` shows, so a person who
    // authorizes this code has authorized the agent that runs later.
    let public_key = load_or_create_agent_keypair()?.public_key();

    // Get hostname for the agent's name
    let hostname = std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "localhost".to_string());

    // No endpoint: nothing listens yet, so the link carries no `rpc`.
    let short_id = &public_key.to_base64()[..7.min(public_key.to_base64().len())];
    let descriptor = AgentDescriptor::new(
        public_key,
        String::new(),
        Some(format!("{hostname}._a2a._tcp.local.")),
        short_id.to_string(),
    )
    .with_name(&hostname)
    .with_entitlements(default_entitlements());

    // Display authorization QR code with DID:key
    display_authorization_qr(&descriptor)?;

    println!();

    Ok(())
}
