//! The keys an agent holds as itself (#766): the workload key its account
//! is, and the keys it uses for gossip, TDF and iroh, each kept in the
//! agent's directory and vouched for by the workload key (INGRESS-005).

use std::sync::Arc;

use arkavo_device_identity::DeviceIdentityError;
use arkavo_device_identity::subordinate_key::{self, Purpose, SubordinateKey, Voucher};
use arkavo_device_identity::workload_key::{self, WorkloadKey};
use arkavo_server::A2aServer;

pub(super) struct AgentKeys {
    /// Made in the Secure Enclave where this Mac has one. It signs the agent
    /// card; the identity provider moves to it with authnz-rs#102.
    pub(super) workload: Arc<WorkloadKey>,
    /// Kept across restarts, so peers that bound it once bind it again
    /// rather than refuse a new key under the same name (INGRESS-017).
    pub(super) gossip: SubordinateKey,
    pub(super) gossip_voucher: Arc<Voucher>,
    #[cfg(feature = "kas")]
    tdf: Arc<SubordinateKey>,
    #[cfg(feature = "iroh")]
    iroh: Arc<SubordinateKey>,
    /// One per key above but the workload key, for the card to carry.
    vouchers: Vec<Arc<Voucher>>,
}

impl AgentKeys {
    /// The keys of the agent named `agent`, each made on first use.
    pub(super) fn load_or_create(agent: &str) -> Result<Self, DeviceIdentityError> {
        let workload = Arc::new(workload_key::load_or_create(agent)?);
        let vouch = |key: &SubordinateKey| Arc::new(Voucher::new(workload.clone(), key));

        let gossip = subordinate_key::load_or_create(agent, Purpose::Gossip)?;
        let gossip_voucher = vouch(&gossip);
        #[cfg(feature = "kas")]
        let tdf = Arc::new(subordinate_key::load_or_create(agent, Purpose::Tdf)?);
        #[cfg(feature = "iroh")]
        let iroh = Arc::new(subordinate_key::load_or_create(agent, Purpose::Iroh)?);
        let vouchers = vec![
            gossip_voucher.clone(),
            #[cfg(feature = "kas")]
            vouch(&tdf),
            #[cfg(feature = "iroh")]
            vouch(&iroh),
        ];
        Ok(Self {
            workload,
            gossip,
            gossip_voucher,
            #[cfg(feature = "kas")]
            tdf,
            #[cfg(feature = "iroh")]
            iroh,
            vouchers,
        })
    }

    /// Give the server the keys it signs, rewraps and binds iroh with, and
    /// the statements its card carries.
    pub(super) async fn install(&self, server: &A2aServer) {
        server.set_workload_key(self.workload.clone()).await;
        server.set_key_vouchers(self.vouchers.clone()).await;
        #[cfg(feature = "kas")]
        server.set_kas_key(self.tdf.clone()).await;
        #[cfg(feature = "iroh")]
        server.set_iroh_key(self.iroh.clone()).await;
    }
}
