//! Roles an orchestrator assigns to this agent (SwarmKit draft-02 §7,
//! swarmkit-delegation.spec.yaml).
//!
//! An agent started without a kit is a specialist: the orchestrator it is
//! bound to can give it a kit role. The role sets what the agent tells its
//! model, which tools it may use and whether it serves image generation.
//! When the role ends, by revocation, expiry or release, the agent goes
//! back to what it was when it started.

use std::path::PathBuf;
use std::sync::{Arc, Weak};

use arkavo_swarmkit::IMAGE_GENERATION;
use arkavo_swarmkit_delegation::{
    CapabilityReport, Delegation, Policy, REFUSAL_CODE, Ready, Refusal, RefusalReason, RevokeError,
    Revoked, Specialist, SpecialistStatus, StateStore, StoreError,
};
use chrono::Utc;
use jsonrpsee::types::ErrorObjectOwned;
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use super::assets::{AssetService, Hosting};
use super::config_helpers::AgentMetadata;

/// The model this host runs for a kit model family and size, or `None`
/// when it has none.
pub type ModelResolver = Arc<dyn Fn(&str, Option<&str>) -> Option<String> + Send + Sync>;

/// How the CLI sets this agent up as a specialist.
pub struct SpecialistConfig {
    /// The agent's own did:key, which envelopes are addressed to.
    pub key: String,
    /// Where the binding and the active delegation are kept, beside the key.
    pub state_dir: PathBuf,
    /// False for an agent started with a kit role, whose operator chose
    /// its role.
    pub enlistable: bool,
    /// The orchestrator the operator named with `--orchestrator`.
    pub operator_orchestrator: Option<String>,
    pub resolve_model: ModelResolver,
}

/// What the agent was before any role, restored when a role ends.
struct Baseline {
    purpose: String,
    model: String,
    description: Option<String>,
    serving: bool,
}

pub struct SpecialistService {
    specialist: Mutex<Specialist>,
    enlistable: bool,
    metadata: Arc<RwLock<AgentMetadata>>,
    assets: Option<Arc<AssetService>>,
    baseline: Baseline,
    resolve_model: ModelResolver,
    expiry: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl SpecialistService {
    /// Load this agent's binding and role, apply a role it still holds, and
    /// name the operator's orchestrator if there is one.
    pub async fn open(
        config: SpecialistConfig,
        metadata: Arc<RwLock<AgentMetadata>>,
        assets: Option<Arc<AssetService>>,
    ) -> Result<Arc<Self>, StoreError> {
        let now = Utc::now();
        let policy = Policy {
            first_use: config.enlistable,
        };
        let mut specialist =
            Specialist::open(config.key, StateStore::new(config.state_dir), policy, now)?;
        if let Some(orchestrator) = &config.operator_orchestrator {
            specialist.bind_operator(orchestrator, now)?;
        }
        let baseline = {
            let meta = metadata.read().await;
            Baseline {
                purpose: meta.purpose.clone(),
                model: meta.model.clone(),
                description: meta.description.clone(),
                serving: meta.image_generation == Hosting::WhereQualified,
            }
        };
        let service = Arc::new(Self {
            specialist: Mutex::new(specialist),
            enlistable: config.enlistable,
            metadata,
            assets,
            baseline,
            resolve_model: config.resolve_model,
            expiry: std::sync::Mutex::new(None),
        });
        service.resume().await?;
        Ok(service)
    }

    /// Take up the role a restart left in place, unless the host can no
    /// longer take it.
    async fn resume(self: &Arc<Self>) -> Result<(), StoreError> {
        let mut specialist = self.specialist.lock().await;
        if let Some(binding) = specialist.binding() {
            info!(orchestrator = %binding.orchestrator, mode = ?binding.mode, "takes roles from its bound orchestrator");
        } else if self.enlistable {
            info!(
                "takes a role from the first orchestrator that assigns one (first-use); \
                 `arkavo agent --orchestrator <did:key>` names one instead"
            );
        }
        let Some(delegation) = specialist.active().filter(|_| self.enlistable).cloned() else {
            return Ok(());
        };
        match self.check_host(&delegation) {
            Ok(model) => {
                self.apply(&delegation, model).await;
                info!(role = %delegation.role_id, flight = %delegation.flight_id, "resumed its role");
                self.schedule_expiry(&delegation);
            }
            Err(refusal) => {
                warn!(role = %delegation.role_id, %refusal, "dropped a role this host can no longer take");
                specialist.abandon()?;
            }
        }
        Ok(())
    }

    /// `swarmkit/status` (§7.3.4).
    pub async fn status(&self) -> SpecialistStatus {
        let capabilities = self
            .assets
            .as_ref()
            .and_then(|a| a.generation_capability())
            .map(|ready| CapabilityReport {
                name: IMAGE_GENERATION.to_string(),
                ready,
            })
            .into_iter()
            .collect();
        let mut status = self.specialist.lock().await.status(capabilities, 0.0);
        if !self.enlistable {
            // A kit agent shares its key's state with other agents on the
            // host but takes no role from it.
            status.binding = None;
            status.delegation = None;
        }
        status
    }

    /// `swarmkit/delegate` (§7.3).
    pub async fn delegate(self: &Arc<Self>, envelope: Value) -> Result<Ready, ErrorObjectOwned> {
        if !self.enlistable {
            return Err(refused(&Refusal::new(
                RefusalReason::BindingRequired,
                "this agent runs the role its kit names",
            )));
        }
        let now = Utc::now();
        let mut specialist = self.specialist.lock().await;
        let admitted = specialist.admit(envelope, now).map_err(|r| refused(&r))?;
        let delegation = admitted.delegation().clone();
        let model = self.check_host(&delegation).map_err(|r| refused(&r))?;
        self.apply(&delegation, model).await;
        let ready = match specialist.commit(admitted, now) {
            Ok(ready) => ready,
            Err(e) => {
                self.restore(&specialist).await;
                return Err(internal(&e));
            }
        };
        drop(specialist);
        info!(
            role = %delegation.role_id,
            flight = %delegation.flight_id,
            orchestrator = %delegation.orchestrator,
            expires = %delegation.expires,
            "took a role from its orchestrator"
        );
        self.schedule_expiry(&delegation);
        Ok(ready)
    }

    /// `swarmkit/revoke` (§7.4).
    pub async fn revoke(&self, revocation: Value) -> Result<Revoked, ErrorObjectOwned> {
        let mut specialist = self.specialist.lock().await;
        let (revoked, changed) =
            specialist
                .revoke(revocation, Utc::now())
                .map_err(|e| match e {
                    RevokeError::Refused(refusal) => refused(&refusal),
                    RevokeError::Store(e) => internal(&e),
                })?;
        if changed {
            self.restore(&specialist).await;
            info!(
                bound = specialist.binding().is_some(),
                "its orchestrator revoked its role"
            );
        }
        Ok(revoked)
    }

    /// Whether this host can take `delegation`, and the model it runs it
    /// with, `None` when the role names none (§7.3 step 6).
    fn check_host(&self, delegation: &Delegation) -> Result<Option<String>, Refusal> {
        let unsuitable = |detail: String| Refusal::new(RefusalReason::HostUnsuitable, detail);
        for capability in &delegation.host_requirements {
            if capability != IMAGE_GENERATION {
                return Err(unsuitable(format!("unknown capability {capability}")));
            }
            let generates = self
                .assets
                .as_ref()
                .and_then(|a| a.generation_capability())
                .is_some();
            if !generates {
                return Err(unsuitable("this host cannot generate images".into()));
            }
        }
        if !delegation.context.initial_inputs.is_empty() {
            return Err(unsuitable("initial_inputs are not supported".into()));
        }
        let Some(model) = &delegation.agent_provisioning.model else {
            return Ok(None);
        };
        (self.resolve_model)(&model.family, model.size.as_deref())
            .map(Some)
            .ok_or_else(|| {
                unsuitable(format!(
                    "no model for family {:?}, size {:?}",
                    model.family, model.size
                ))
            })
    }

    async fn apply(&self, delegation: &Delegation, model: Option<String>) {
        {
            let mut meta = self.metadata.write().await;
            meta.purpose = delegation.purpose();
            meta.model = model.unwrap_or_else(|| self.baseline.model.clone());
            meta.description
                .clone_from(&delegation.context.role_description);
            meta.granted_tools = delegation.granted_tools();
            meta.specialized = true;
        }
        if let Some(assets) = &self.assets {
            let serves = delegation
                .host_requirements
                .iter()
                .any(|c| c == IMAGE_GENERATION);
            assets.set_serving(serves);
        }
    }

    /// Make the agent match what `specialist` now holds: its role, or the
    /// agent it was before any role.
    async fn restore(&self, specialist: &Specialist) {
        if let Some(delegation) = specialist.active().cloned()
            && let Ok(model) = self.check_host(&delegation)
        {
            self.apply(&delegation, model).await;
            return;
        }
        {
            let mut meta = self.metadata.write().await;
            meta.purpose.clone_from(&self.baseline.purpose);
            meta.model.clone_from(&self.baseline.model);
            meta.description.clone_from(&self.baseline.description);
            meta.granted_tools.clear();
            meta.specialized = false;
        }
        if let Some(assets) = &self.assets {
            assets.set_serving(self.baseline.serving);
        }
    }

    /// End the role when its delegation expires, unless a later one
    /// replaced it by then.
    fn schedule_expiry(self: &Arc<Self>, delegation: &Delegation) {
        let service: Weak<Self> = Arc::downgrade(self);
        let expires = delegation.expires;
        let wait = (expires - Utc::now()).to_std().unwrap_or_default();
        let task = tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            let Some(service) = service.upgrade() else {
                return;
            };
            let mut specialist = service.specialist.lock().await;
            match specialist.expire(Utc::now()) {
                Ok(true) => {
                    service.restore(&specialist).await;
                    info!(%expires, "its role expired");
                }
                Ok(false) => {}
                Err(e) => warn!(error = %e, "could not record that its role expired"),
            }
        });
        let previous = self
            .expiry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(task);
        if let Some(previous) = previous {
            previous.abort();
        }
    }
}

fn refused(refusal: &Refusal) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(REFUSAL_CODE, refusal.message(), Some(refusal))
}

fn internal(error: &StoreError) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(-32603, "could not record the role", Some(error.to_string()))
}

#[cfg(test)]
mod tests;
