//! One-shot helper: signs the three vrm-production-kit skills with a
//! deterministic dev key and prints the YAML-pasteable payload + sig.
//!
//! Run: `cargo run -p arkavo-swarmkit-runtime --example sign_vrm_production_skills`

use arkavo_swarmkit::SkillContent;
use arkavo_swarmkit_runtime::sign_skill_content;
use ed25519_dalek::SigningKey;

fn main() {
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let did = "did:web:arkavo.com";

    let skills = [
        SkillContent {
            name: "prompt-design".into(),
            description: "Produce an avatar specification from a creator brief.".into(),
            instructions: "From the creator_brief input, produce a structured avatar specification: visual brief (face, body, costume), persona (voice, mannerisms), constraints (target_platform performance budget, accessibility requirements). Cite the creator_brief sentence ranges supporting each spec choice.".into(),
            resources: vec![],
        },
        SkillContent {
            name: "vrm-assemble".into(),
            description: "Emit VRM 1.0 metadata JSON from an avatar specification.".into(),
            instructions: "Translate the prompt_designer's avatar specification into VRM 1.0 schema JSON: skeleton (humanoid bones), blendshapes (expression presets), and metadata (license, author, version). Do not emit binary glTF/VRM data — JSON metadata only.".into(),
            resources: vec![],
        },
        SkillContent {
            name: "vrm-validate".into(),
            description: "Validate VRM 1.0 spec compliance and prompt fidelity.".into(),
            instructions: "Validate the assembled VRM JSON against the VRM 1.0 schema (required humanoid bones, expression presets, metadata). Score prompt fidelity by comparing the assembled spec against the prompt_designer's specification. Flag missing required fields and accessibility gaps.".into(),
            resources: vec![],
        },
    ];

    println!(
        "# vrm-production-kit signed skills (regenerate via `cargo run -p arkavo-swarmkit-runtime --example sign_vrm_production_skills`)"
    );
    println!("# signer DID: {did}");
    for content in skills.iter() {
        let signed = sign_skill_content(content, did, &key);
        println!("\n## skill: {}", content.name);
        println!("payload: {}", serde_json::to_string(content).unwrap());
        println!("signature: {}", signed.signature_b64url);
        println!("signed_by: {}", signed.signed_by);
    }
}
