//! The harness configuration (`config.toml`).
//!
//! It names files, never key bytes. Relative paths are resolved against the configuration
//! file's directory. Values the implemented profile fixes (algorithm, lifetimes, skew) are
//! checked for agreement rather than silently overridden, and anything that would make this
//! harness look like more than a local test environment is refused.

use anyhow::{Context, bail};
use arkavo_tap::{Environment, ExternalOrigin, Profile, TrustAuthority};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DemoConfig {
    pub environment: String,
    pub profile: String,
    pub merchant: MerchantSection,
    pub trust: TrustSection,
    pub signature: SignatureSection,
    pub replay: ReplaySection,
    pub authorization: AuthorizationSection,
    pub keys: KeysSection,
    #[serde(skip)]
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MerchantSection {
    pub listen: String,
    pub external_origin: String,
    pub payment_mode: String,
    pub database: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustSection {
    pub authority: String,
    pub registry_origin: String,
    pub registry_listen: String,
    pub registry_database: String,
    pub allow_loopback_http: bool,
    pub key_max_age_seconds: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignatureSection {
    pub algorithm: String,
    pub outbound_lifetime_seconds: i64,
    pub maximum_lifetime_seconds: i64,
    pub future_time_tolerance_seconds: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplaySection {
    pub backend: String,
    pub path: String,
    pub cleanup_margin_seconds: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationSection {
    pub mode: String,
    pub require_for_order_commit: bool,
    pub issuer_key_id: String,
}

/// Key file locations and the IDs their public halves are registered under.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeysSection {
    pub agent: String,
    pub agent_key_id: String,
    pub other_agent: String,
    pub other_agent_key_id: String,
    pub merchant: String,
    pub consent_issuer: String,
    pub grant: String,
}

impl DemoConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut config: DemoConfig =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        config.dir = path
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.environment != "local" {
            bail!("this harness runs only in the local environment");
        }
        if self.merchant.payment_mode != "simulated" {
            bail!("payment_mode must be simulated: this harness has no real payment provider");
        }
        if self.authorization.mode == "ap2" {
            bail!("AP2 authorization is not available yet: use local-test-grant");
        }
        if self.authorization.mode != "local-test-grant"
            || !self.authorization.require_for_order_commit
        {
            bail!("order commitment requires a local-test-grant");
        }
        if self.replay.backend != "sqlite" {
            bail!("the replay store must be durable: backend = \"sqlite\"");
        }
        let profile = self.profile()?;
        let s = &self.signature;
        if s.algorithm != profile.algorithm()
            || s.maximum_lifetime_seconds != profile.max_lifetime()
            || s.future_time_tolerance_seconds != profile.future_tolerance()
        {
            bail!(
                "[signature] must match {profile}: algorithm {}, maximum_lifetime_seconds {}, future_time_tolerance_seconds {}",
                profile.algorithm(),
                profile.max_lifetime(),
                profile.future_tolerance()
            );
        }
        if !(1..=s.maximum_lifetime_seconds).contains(&s.outbound_lifetime_seconds) {
            bail!(
                "outbound_lifetime_seconds must be 1 to {}",
                s.maximum_lifetime_seconds
            );
        }
        if !(1..=3600).contains(&self.trust.key_max_age_seconds) {
            bail!("key_max_age_seconds must be 1 to 3600");
        }
        self.origin()?;
        ExternalOrigin::parse(&self.trust.registry_origin).map_err(anyhow::Error::msg)?;
        Ok(())
    }

    pub fn profile(&self) -> anyhow::Result<Profile> {
        Profile::parse(&self.profile).map_err(anyhow::Error::msg)
    }

    pub fn origin(&self) -> anyhow::Result<ExternalOrigin> {
        ExternalOrigin::parse(&self.merchant.external_origin).map_err(anyhow::Error::msg)
    }

    pub fn authority(&self) -> TrustAuthority {
        TrustAuthority {
            name: self.trust.authority.clone(),
            environment: Environment::Local,
        }
    }

    /// `relative` resolved against the configuration directory.
    pub fn path(&self, relative: &str) -> PathBuf {
        self.dir.join(relative)
    }
}

/// The configuration `init` writes, for a merchant on `merchant_port` and a registry on
/// `registry_port`.
pub fn default_toml(merchant_port: u16, registry_port: u16) -> String {
    format!(
        r#"# Development-only configuration for the local TAP harness. Nothing here is a production
# credential or a Visa registration: the registry below is a local trust authority.
environment = "local"
profile = "arkavo-rfc9421-v1"

[merchant]
listen = "127.0.0.1:{merchant_port}"
external_origin = "http://127.0.0.1:{merchant_port}"
payment_mode = "simulated"
database = "merchant.sqlite"

[trust]
authority = "arkavo-local-demo"
registry_origin = "http://127.0.0.1:{registry_port}"
registry_listen = "127.0.0.1:{registry_port}"
registry_database = "registry.sqlite"
allow_loopback_http = true
key_max_age_seconds = 60

[signature]
algorithm = "ed25519"
outbound_lifetime_seconds = 120
maximum_lifetime_seconds = 480
future_time_tolerance_seconds = 0

[replay]
backend = "sqlite"
path = "replay.sqlite"
cleanup_margin_seconds = 30

[authorization]
mode = "local-test-grant"
require_for_order_commit = true
issuer_key_id = "consent-issuer"

# Key files (mode 600), never key bytes.
[keys]
agent = "keys/agent-a.key"
agent_key_id = "agent-a"
other_agent = "keys/agent-b.key"
other_agent_key_id = "agent-b"
merchant = "keys/merchant.key"
consent_issuer = "keys/consent-issuer.key"
grant = "approval.signed.json"
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> anyhow::Result<DemoConfig> {
        let c: DemoConfig = toml::from_str(text)?;
        c.validate()?;
        Ok(c)
    }

    #[test]
    fn the_default_configuration_is_valid() {
        let c = parsed(&default_toml(3001, 9002)).unwrap();
        assert_eq!(c.origin().unwrap().as_origin(), "http://127.0.0.1:3001");
        assert_eq!(c.profile().unwrap(), Profile::ArkavoRfc9421V1);
        assert_eq!(c.authority().environment, Environment::Local);
        assert_eq!(c.path("x"), PathBuf::from("x"));
    }

    #[test]
    fn refuses_anything_beyond_a_local_simulation() {
        let base = default_toml(3001, 9002);
        for (from, to) in [
            ("environment = \"local\"", "environment = \"production\""),
            ("payment_mode = \"simulated\"", "payment_mode = \"live\""),
            ("mode = \"local-test-grant\"", "mode = \"ap2\""),
            ("mode = \"local-test-grant\"", "mode = \"none\""),
            (
                "require_for_order_commit = true",
                "require_for_order_commit = false",
            ),
            ("backend = \"sqlite\"", "backend = \"memory\""),
            (
                "profile = \"arkavo-rfc9421-v1\"",
                "profile = \"visa-sample-fixture\"",
            ),
            ("algorithm = \"ed25519\"", "algorithm = \"rsa-pss-sha512\""),
            (
                "future_time_tolerance_seconds = 0",
                "future_time_tolerance_seconds = 30",
            ),
            (
                "outbound_lifetime_seconds = 120",
                "outbound_lifetime_seconds = 600",
            ),
            ("key_max_age_seconds = 60", "key_max_age_seconds = 0"),
            (
                "external_origin = \"http://127.0.0.1:3001\"",
                "external_origin = \"http://127.0.0.1:3001/shop\"",
            ),
        ] {
            let text = base.replace(from, to);
            assert!(parsed(&text).is_err(), "{to}");
        }
        assert!(parsed(&format!("{base}\nunknown = 1\n")).is_err());
    }
}
