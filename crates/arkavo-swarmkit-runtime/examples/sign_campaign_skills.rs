//! Signs the inline skills of a kit file with the deterministic dev key and
//! prints the YAML-pasteable payload + signature for each.
//!
//! Run: `cargo run -p arkavo-swarmkit-runtime --example sign_campaign_skills -- examples/campaign-kit/campaign-kit.swarmkit.yaml`
//!
//! The kit path is required so that a copy of the kit is signed from its own
//! text, not from the example it was copied from.

#[path = "support/sign_kit_skills.rs"]
mod sign_kit_skills;

fn main() -> std::process::ExitCode {
    sign_kit_skills::run("sign_campaign_skills")
}
