//! One-shot helper: signs the three code-review-kit skills with a
//! deterministic dev key and prints the YAML-pasteable payload + sig.
//!
//! Run: `cargo run -p arkavo-swarmkit-runtime --example sign_code_review_skills`

use arkavo_swarmkit::SkillContent;
use arkavo_swarmkit_runtime::sign_skill_content;
use ed25519_dalek::SigningKey;

fn main() {
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let did = "did:web:arkavo.com";

    let skills = [
        SkillContent {
            name: "code-review".into(),
            description: "Review a code diff for correctness, design, and naming.".into(),
            instructions: "Read the diff against target_branch. Identify correctness defects, design concerns, naming issues, and unclear logic. Reference file:line for each finding. Score each finding's severity (low / medium / high).".into(),
            resources: vec![],
        },
        SkillContent {
            name: "security-audit".into(),
            description: "Audit a code diff for security defects per OWASP / CWE.".into(),
            instructions: "Scan the diff for OWASP Top 10 issues (injection, broken auth, exposure, XXE, broken access, misconfiguration, XSS, deserialization, components-with-known-vulns, logging gaps) and any CWE-class defects. Output structured findings with CWE/CVE references where applicable.".into(),
            resources: vec![],
        },
        SkillContent {
            name: "test-write".into(),
            description: "Write tests for changed code paths.".into(),
            instructions: "For each changed function or module in the diff, write at least one unit test covering the new behavior and one negative-path test. Use the language conventions identified by the input.language field.".into(),
            resources: vec![],
        },
    ];

    println!(
        "# code-review-kit signed skills (regenerate via `cargo run -p arkavo-swarmkit-runtime --example sign_code_review_skills`)"
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
