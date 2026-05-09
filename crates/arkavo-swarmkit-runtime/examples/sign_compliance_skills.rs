//! One-shot helper: signs the three compliance-kit skills with a
//! deterministic dev key and prints the YAML-pasteable payload + sig.
//!
//! Run: `cargo run -p arkavo-swarmkit-runtime --example sign_compliance_skills`

use arkavo_swarmkit::SkillContent;
use arkavo_swarmkit_runtime::sign_skill_content;
use ed25519_dalek::SigningKey;

fn main() {
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let did = "did:web:arkavo.com";

    let skills = [
        SkillContent {
            name: "pii-classify".into(),
            description: "Classify documents for PII per jurisdiction.".into(),
            instructions: "Scan the document input for PII (US-CA CCPA categories, EU GDPR Article 4(1) categories, US HIPAA PHI). For each finding, output { category, jurisdiction_basis, document_offset, confidence }. Optimize for recall — false negatives are worse than false positives in this domain.".into(),
            resources: vec![],
        },
        SkillContent {
            name: "policy-enforce".into(),
            description: "Apply jurisdiction-aware redaction or escalation.".into(),
            instructions: "For each PII finding, apply the jurisdiction-specific policy: redact (replace with category placeholder), escalate (route to human reviewer), or pass-through (per consent_status). Refuse the document if data_subject_consent_status is incompatible with the document's classification.".into(),
            resources: vec![],
        },
        SkillContent {
            name: "audit-trail".into(),
            description: "Produce an audit-ready compliance report.".into(),
            instructions: "Log every classification finding and policy action with timestamp + role + jurisdiction_basis. Identify gaps: PII categories the policy didn't cover, escalation paths that weren't taken. Output a structured compliance_report deliverable suitable for regulator review.".into(),
            resources: vec![],
        },
    ];

    println!(
        "# compliance-kit signed skills (regenerate via `cargo run -p arkavo-swarmkit-runtime --example sign_compliance_skills`)"
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
