//! Shared body of the `sign_*_skills` examples.
//!
//! The skill text comes from the kit file named on the command line. An
//! earlier version kept its own copy of each kit's skills, so after an
//! author edited the YAML it went on signing the old text and the pasted
//! signatures failed verification.
//!
//! The signing key is a fixed development key. Signatures made with it
//! prove nothing about who signed; they exist so the example kits verify
//! offline against a resolver that is given the matching public key.

use std::fmt::Write as _;
use std::process::ExitCode;

use arkavo_swarmkit::kit_skills_from_yaml;
use arkavo_swarmkit_runtime::sign_skill_content;
use ed25519_dalek::SigningKey;

const DEV_KEY_BYTES: [u8; 32] = [7u8; 32];
const DEV_SIGNER_DID: &str = "did:web:arkavo.com";

pub fn run(example: &str) -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (Some(path), None) = (args.next(), args.next()) else {
        eprintln!(
            "usage: cargo run -p arkavo-swarmkit-runtime --example {example} -- <kit.swarmkit.yaml>"
        );
        return ExitCode::from(2);
    };
    let yaml = match std::fs::read_to_string(&path) {
        Ok(yaml) => yaml,
        Err(err) => {
            eprintln!("read {path}: {err}");
            return ExitCode::from(2);
        }
    };
    match render(example, &path, &yaml) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{path}: {err}");
            ExitCode::FAILURE
        }
    }
}

/// The YAML-pasteable payload and signature for every inline skill in
/// `yaml`, signed with the development key.
fn render(example: &str, path: &str, yaml: &str) -> Result<String, String> {
    let skills = kit_skills_from_yaml(yaml).map_err(|err| err.to_string())?;
    let key = SigningKey::from_bytes(&DEV_KEY_BYTES);

    let mut out = String::new();
    let _ = writeln!(
        out,
        "# signed skills from {path} (regenerate via `cargo run -p arkavo-swarmkit-runtime \
         --example {example} -- {path}`)"
    );
    let _ = writeln!(out, "# signer DID: {DEV_SIGNER_DID}");

    let mut signed_count = 0;
    for skill in &skills {
        let _ = writeln!(
            out,
            "\n## role {}: {} {}",
            skill.role_id, skill.id, skill.version
        );
        let Some(content) = &skill.content else {
            let _ = writeln!(
                out,
                "# not signed: the kit carries no inline payload for it"
            );
            continue;
        };
        let signed = sign_skill_content(content, DEV_SIGNER_DID, &key);
        let payload = serde_json::to_string(content).map_err(|err| err.to_string())?;
        let _ = writeln!(out, "payload: {payload}");
        let _ = writeln!(out, "signature: {}", signed.signature_b64url);
        let _ = writeln!(out, "signed_by: {}", signed.signed_by);
        signed_count += 1;
    }
    if signed_count == 0 {
        return Err("no inline skills to sign".to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAMPAIGN_KIT: &str =
        include_str!("../../../../examples/campaign-kit/campaign-kit.swarmkit.yaml");
    const ORIGINAL: &str = "Extract three to five selling points relevant to the target platform.";
    const EDITED: &str = "Extract exactly two selling points relevant to the target platform.";

    fn signature_lines(output: &str) -> Vec<&str> {
        output
            .lines()
            .filter(|l| l.starts_with("signature: "))
            .collect()
    }

    /// The development key and signer must keep producing the signatures
    /// the shipped kit carries, or every example kit stops verifying.
    #[test]
    fn signatures_for_the_shipped_kit_match_the_ones_in_the_file() {
        let output = render("sign_campaign_skills", "kit.yaml", CAMPAIGN_KIT).unwrap();
        let signatures = signature_lines(&output);
        assert_eq!(signatures.len(), 3);
        for line in signatures {
            let signature = line.trim_start_matches("signature: ");
            assert!(
                CAMPAIGN_KIT.contains(&format!("signature: \"{signature}\"")),
                "kit does not carry {signature}"
            );
        }
        assert!(output.contains("signed_by: did:web:arkavo.com"));
    }

    /// Regression: the example signed a copy of the skill text compiled
    /// into it, so an edit to the kit file changed nothing it printed.
    #[test]
    fn editing_a_skill_in_the_file_changes_what_is_signed() {
        assert!(CAMPAIGN_KIT.contains(ORIGINAL), "fixture text moved");
        let edited_kit = CAMPAIGN_KIT.replace(ORIGINAL, EDITED);

        let before = render("sign_campaign_skills", "kit.yaml", CAMPAIGN_KIT).unwrap();
        let after = render("sign_campaign_skills", "kit.yaml", &edited_kit).unwrap();

        assert!(after.contains(EDITED), "{after}");
        assert!(!after.contains(ORIGINAL), "{after}");
        let (before, after) = (signature_lines(&before), signature_lines(&after));
        assert_ne!(before[0], after[0], "the edited skill must be re-signed");
        assert_eq!(
            before[1..],
            after[1..],
            "untouched skills keep their signatures"
        );
    }

    /// The edit above leaves the declared `kit.id` stale. Signing happens
    /// before the id is recomputed, so that must not stop it.
    #[test]
    fn a_stale_kit_id_does_not_block_signing() {
        let edited_kit = CAMPAIGN_KIT.replace(ORIGINAL, EDITED);
        assert!(arkavo_swarmkit::parse_yaml(&edited_kit).is_err());
        assert!(render("sign_campaign_skills", "kit.yaml", &edited_kit).is_ok());
    }

    #[test]
    fn a_kit_with_nothing_to_sign_is_an_error() {
        assert!(
            CAMPAIGN_KIT.contains("source: \"inline\""),
            "fixture text moved"
        );
        let by_reference = CAMPAIGN_KIT.replace("source: \"inline\"", "source: \"registry\"");
        assert_eq!(
            render("sign_campaign_skills", "kit.yaml", &by_reference),
            Err("no inline skills to sign".to_string())
        );
    }
}
