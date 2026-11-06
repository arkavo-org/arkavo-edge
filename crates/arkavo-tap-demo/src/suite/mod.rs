//! `verify-suite`: the acceptance scenarios, end to end.
//!
//! A registry and a merchant start in this process, in a fresh temporary directory, with an
//! injectable clock and independent clients. The suite never touches the configured
//! `.tap-demo` state.

mod scenarios;

use crate::buyer::Buyer;
use crate::config::DemoConfig;
use crate::merchant::Merchant;
use crate::registry::Registration;
use crate::server::{self, SharedClock};
use crate::{init, keyfile};
use anyhow::{Context, bail};
use arkavo_tap::{Clock as _, LocalTestGrant, ManualClock, SignedEnvelope, SystemClock};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{SigningKey, VerifyingKey};
use std::sync::Arc;
use tokio::task::JoinHandle;

/// How a scenario is covered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Coverage {
    /// Run end to end here.
    Ran(Result<(), String>),
    /// Covered by the named crate tests, which need a lower-level seam than HTTP.
    Elsewhere(&'static str),
    /// The capability is disabled; the reason says why.
    Disabled(&'static str),
}

#[derive(Debug, Clone)]
pub struct Line {
    pub id: &'static str,
    pub title: &'static str,
    pub coverage: Coverage,
}

/// A running registry and merchant, two registered agents and an unregistered one.
pub struct Env {
    _dir: tempfile::TempDir,
    pub config: DemoConfig,
    pub clock: ManualClock,
    pub a: Buyer,
    pub b: Buyer,
    pub stranger: Buyer,
    pub merchant: Arc<Merchant>,
    pub merchant_key: VerifyingKey,
    pub grant: SignedEnvelope,
    issuer: SigningKey,
    agent_key: SigningKey,
    stop_registry: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    registry_task: std::sync::Mutex<Option<JoinHandle<()>>>,
    merchant_task: JoinHandle<()>,
}

impl Env {
    pub async fn start() -> anyhow::Result<Self> {
        let dir = tempfile::tempdir()?;
        let registry_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let merchant_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let now = SystemClock.now();
        let config = init::initialize(
            dir.path(),
            merchant_listener.local_addr()?.port(),
            registry_listener.local_addr()?.port(),
            now,
        )?;
        let clock = ManualClock::new(now);
        let shared: SharedClock = Arc::new(clock.clone());
        let registry = server::registry(&config, shared.clone()).await?;
        let (stop_registry, stopped) = tokio::sync::oneshot::channel::<()>();
        let registry_task = tokio::spawn(async move {
            let shutdown = async {
                let _ = stopped.await;
            };
            let _ = crate::registry::serve(registry_listener, registry, shutdown).await;
        });
        let (merchant, app) = server::merchant(&config, shared.clone()).await?;
        let merchant_task = tokio::spawn(async move {
            let _ = axum::serve(merchant_listener, app).await;
        });
        keyfile::create(&dir.path().join("keys/agent-c.key"))?;
        let k = &config.keys;
        let a = Buyer::from_config(&config, &k.agent, &k.agent_key_id, shared.clone())?;
        let b = Buyer::from_config(
            &config,
            &k.other_agent,
            &k.other_agent_key_id,
            shared.clone(),
        )?;
        let stranger = Buyer::from_config(&config, "keys/agent-c.key", "agent-c", shared.clone())?;
        let grant: SignedEnvelope =
            serde_json::from_str(&std::fs::read_to_string(config.path(&k.grant))?)?;
        let env = Env {
            merchant_key: keyfile::load(&config.path(&k.merchant))?.verifying_key(),
            issuer: keyfile::load(&config.path(&k.consent_issuer))?,
            agent_key: keyfile::load(&config.path(&k.agent))?,
            _dir: dir,
            config,
            clock,
            a,
            b,
            stranger,
            merchant,
            grant,
            stop_registry: std::sync::Mutex::new(Some(stop_registry)),
            registry_task: std::sync::Mutex::new(Some(registry_task)),
            merchant_task,
        };
        register(&env.config, &env.a, Some("runtime-a")).await?;
        register(&env.config, &env.b, None).await?;
        Ok(env)
    }

    /// A grant from the configured issuer, as `init` would issue it, changed by `change`.
    pub fn mint(&self, change: impl FnOnce(&mut LocalTestGrant)) -> anyhow::Result<SignedEnvelope> {
        let mut grant = init::grant_for(&self.config, &self.agent_key, self.clock.now());
        change(&mut grant);
        init::seal(&self.config, &self.issuer, &grant)
    }

    pub async fn revoke(&self, key_id: &str) -> anyhow::Result<()> {
        let url = format!(
            "{}/dev/v1/agents/{key_id}/revoke",
            self.config.trust.registry_origin
        );
        let status = reqwest::Client::new().post(url).send().await?.status();
        if !status.is_success() {
            bail!("revoking {key_id}: {status}");
        }
        Ok(())
    }

    /// Stops the registry and waits until it has closed every connection.
    pub async fn stop_registry(&self) -> anyhow::Result<()> {
        let stop = self
            .stop_registry
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?
            .take();
        let task = self
            .registry_task
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?
            .take();
        if let (Some(stop), Some(task)) = (stop, task) {
            let _ = stop.send(());
            tokio::time::timeout(std::time::Duration::from_secs(10), task)
                .await
                .context("the registry did not stop")??;
        }
        Ok(())
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        if let Ok(mut task) = self.registry_task.lock()
            && let Some(task) = task.take()
        {
            task.abort();
        }
        self.merchant_task.abort();
    }
}

/// Registers `buyer`'s public key with the configured registry.
pub async fn register(
    config: &DemoConfig,
    buyer: &Buyer,
    runtime: Option<&str>,
) -> anyhow::Result<()> {
    let registration = Registration {
        key_id: buyer.key_id().to_owned(),
        x: URL_SAFE_NO_PAD.encode(buyer.public_key()),
        provider: "arkavo-local-demo-provider".into(),
        runtime: runtime.map(str::to_owned),
        purposes: vec!["agent-browser-auth".into(), "agent-payer-auth".into()],
        validity_seconds: 7 * 86_400,
    };
    let url = format!("{}/dev/v1/agents", config.trust.registry_origin);
    let response = reqwest::Client::new()
        .post(url)
        .json(&registration)
        .send()
        .await
        .context("reaching the registry")?;
    let status = response.status();
    if !status.is_success() && status != reqwest::StatusCode::CONFLICT {
        bail!("registering {}: {status}", buyer.key_id());
    }
    Ok(())
}

/// Runs every scenario and reports each one.
pub async fn run() -> anyhow::Result<Vec<Line>> {
    let env = Env::start().await?;
    Ok(scenarios::all(&env).await)
}

/// Prints the report; true when nothing ran and failed.
pub fn print(lines: &[Line]) -> bool {
    let mut ok = true;
    for line in lines {
        let (mark, note) = match &line.coverage {
            Coverage::Ran(Ok(())) => ("PASS", String::new()),
            Coverage::Ran(Err(why)) => {
                ok = false;
                ("FAIL", format!(" ({why})"))
            }
            Coverage::Elsewhere(at) => ("UNIT", format!(" ({at})")),
            Coverage::Disabled(why) => ("OFF ", format!(" ({why})")),
        };
        println!("{} {mark} {}{note}", line.id, line.title);
    }
    let ran = lines
        .iter()
        .filter(|l| matches!(l.coverage, Coverage::Ran(_)))
        .count();
    println!(
        "{ran} scenarios run end to end against the local, simulated environment; {}",
        if ok { "all passed" } else { "some FAILED" }
    );
    ok
}
