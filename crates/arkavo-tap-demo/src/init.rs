//! `init`: separate development identities, local trust configuration and a bounded test
//! grant. Every credential it writes is development-only; nothing is ever overwritten.

use crate::config::{DemoConfig, default_toml};
use crate::keyfile;
use anyhow::Context;
use arkavo_tap::{LOCAL_TEST_GRANT, LocalTestGrant, SignedEnvelope, thumbprint};
use ed25519_dalek::SigningKey;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::Path;

/// Seven days: long enough to try the harness, short enough not to linger.
const GRANT_LIFETIME_SECONDS: i64 = 7 * 86_400;

/// Creates `dir` with its configuration, keys and grant.
pub fn initialize(
    dir: &Path,
    merchant_port: u16,
    registry_port: u16,
    now: i64,
) -> anyhow::Result<DemoConfig> {
    std::fs::create_dir_all(dir.join("keys"))
        .with_context(|| format!("creating {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir.join("keys"), std::fs::Permissions::from_mode(0o700))?;
    }
    let config_path = dir.join("config.toml");
    write_new(
        &config_path,
        default_toml(merchant_port, registry_port).as_bytes(),
    )?;
    let config = DemoConfig::load(&config_path)?;
    let k = &config.keys;
    let agent = keyfile::create(&config.path(&k.agent))?;
    keyfile::create(&config.path(&k.other_agent))?;
    keyfile::create(&config.path(&k.merchant))?;
    let issuer = keyfile::create(&config.path(&k.consent_issuer))?;
    let grant = grant_for(&config, &agent, now);
    let sealed = seal(&config, &issuer, &grant)?;
    write_new(
        &config.path(&k.grant),
        serde_json::to_string_pretty(&sealed)?.as_bytes(),
    )?;
    Ok(config)
}

/// The bounded grant `init` issues to the configured agent.
pub fn grant_for(config: &DemoConfig, agent: &SigningKey, now: i64) -> LocalTestGrant {
    LocalTestGrant {
        label: LOCAL_TEST_GRANT.into(),
        environment: "local".into(),
        grant_id: uuid::Uuid::new_v4().to_string(),
        issuer: config.authorization.issuer_key_id.clone(),
        principal: "principal:local-demo-person".into(),
        actor_key_thumbprint: thumbprint(agent.verifying_key().as_bytes()),
        merchant: config.merchant.external_origin.clone(),
        items: vec!["demo-001".into(), "demo-002".into()],
        max_quantity: 2,
        currency: "USD".into(),
        limit_minor: 5_000,
        expires_at: now + GRANT_LIFETIME_SECONDS,
    }
}

/// Signs `grant` as the configured consent issuer.
pub fn seal(
    config: &DemoConfig,
    issuer: &SigningKey,
    grant: &LocalTestGrant,
) -> anyhow::Result<SignedEnvelope> {
    Ok(arkavo_tap::envelope::seal(
        LOCAL_TEST_GRANT,
        &config.authorization.issuer_key_id,
        issuer,
        &serde_json::to_vec(grant)?,
    ))
}

fn write_new(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("creating {} (init never overwrites)", path.display()))?;
    file.write_all(bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_everything_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tap");
        let config = initialize(&root, 3001, 9002, 1_000).unwrap();
        for file in [
            "config.toml",
            "approval.signed.json",
            "keys/agent-a.key",
            "keys/agent-b.key",
            "keys/merchant.key",
            "keys/consent-issuer.key",
        ] {
            assert!(root.join(file).exists(), "{file}");
        }
        let sealed: SignedEnvelope = serde_json::from_str(
            &std::fs::read_to_string(root.join("approval.signed.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(sealed.typ, LOCAL_TEST_GRANT);
        assert_eq!(config.keys.agent_key_id, "agent-a");
        assert!(
            initialize(&root, 3001, 9002, 1_000).is_err(),
            "never overwrites"
        );
    }
}
