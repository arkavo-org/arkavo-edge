//! One-shot helper: signs the three campaign-kit skills with a
//! deterministic dev key and prints the YAML-pasteable payload + sig.
//!
//! Run: `cargo run -p arkavo-swarmkit-runtime --example sign_campaign_skills`

use arkavo_swarmkit::SkillContent;
use arkavo_swarmkit_runtime::sign_skill_content;
use ed25519_dalek::SigningKey;

fn main() {
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let did = "did:web:arkavo.com";

    let skills = [
        SkillContent {
            name: "asset-analysis".into(),
            description: "Summarize the source asset and extract selling points.".into(),
            instructions: "Extract three to five selling points relevant to the target platform. Cite sentence ranges in the source asset for each point.".into(),
            resources: vec![],
        },
        SkillContent {
            name: "platform-copy".into(),
            description: "Generate platform-specific copy from the analyst's selling points.".into(),
            instructions: "Produce posts, scripts, and risk_notes tailored to the target platform's voice and length constraints.".into(),
            resources: vec![],
        },
        SkillContent {
            name: "campaign-critic".into(),
            description: "Score the produced copy against the rubric and flag unsupported claims.".into(),
            instructions: "For each claim in the copy, check it is supported by the analyst's selling points. Score against the rubric dimensions.".into(),
            resources: vec![],
        },
    ];

    println!(
        "# campaign-kit signed skills (regenerate via `cargo run -p arkavo-swarmkit-runtime --example sign_campaign_skills`)"
    );
    println!("# signer DID: {did}");
    for (i, content) in skills.iter().enumerate() {
        let signed = sign_skill_content(content, did, &key);
        println!("\n## skill {i}: {}", content.name);
        println!("payload: {}", serde_json::to_string(content).unwrap());
        println!("signature: {}", signed.signature_b64url);
        println!("signed_by: {}", signed.signed_by);
    }
}
