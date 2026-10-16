//! Members' own reports of what they can do, read over the member channel
//! rather than from what they announce (MESH-010).

// The cache is a private field of `MeshToolsState`; plain `pub` here would
// trip `unreachable_pub`.
#![allow(clippy::redundant_pub_crate)]

use std::collections::HashMap;
use std::time::{Duration, Instant};

use arkavo_protocol::types::AgentCapabilitiesGetResponse;
use tokio::sync::RwLock;

use crate::MeshToolsState;
use crate::peer::{Patience, Peer};

/// How long a member's report is reused before it is read again.
pub const REPORT_TTL: Duration = Duration::from_mins(1);

/// A member that does not answer within this is left out of one round of
/// reports rather than holding up the choice of a member.
const PATIENCE: Patience = Patience {
    timeout_ms: 5_000,
    max_retries: 0,
};

/// A member discovery found, and what it reports it can do.
#[derive(Debug, Clone)]
pub struct Member {
    pub agent_id: String,
    pub address: String,
    pub report: AgentCapabilitiesGetResponse,
}

struct Cached {
    read_at: Instant,
    address: String,
    report: AgentCapabilitiesGetResponse,
}

/// Reports read in the last [`REPORT_TTL`], by agent id.
#[derive(Default)]
pub(crate) struct ReportCache {
    entries: RwLock<HashMap<String, Cached>>,
}

impl MeshToolsState {
    /// The report of every member discovery found, reusing one read within
    /// [`REPORT_TTL`] from the same address. Members that do not answer are
    /// left out.
    pub async fn member_reports(&self) -> Vec<Member> {
        // A copy, so no lock is held while members answer.
        let addresses = self.agent_addresses.read().await.clone();
        let reads = addresses.into_iter().map(|(agent_id, address)| async move {
            let report = self.report(&agent_id, &address).await?;
            Some(Member {
                agent_id,
                address,
                report,
            })
        });
        futures::future::join_all(reads)
            .await
            .into_iter()
            .flatten()
            .collect()
    }

    async fn report(&self, agent_id: &str, address: &str) -> Option<AgentCapabilitiesGetResponse> {
        if let Some(cached) = self.reports.entries.read().await.get(agent_id)
            && cached.address == address
            && cached.read_at.elapsed() < REPORT_TTL
        {
            return Some(cached.report.clone());
        }
        let read = async {
            let peer = Peer::connect(address, agent_id, PATIENCE).await?;
            let report = peer.capabilities().await;
            peer.close().await;
            report
        }
        .await;
        let mut entries = self.reports.entries.write().await;
        match read {
            Ok(report) => {
                entries.insert(
                    agent_id.to_string(),
                    Cached {
                        read_at: Instant::now(),
                        address: address.to_string(),
                        report: report.clone(),
                    },
                );
                Some(report)
            }
            Err(e) => {
                tracing::debug!(agent_id, error = %e, "member did not report its capabilities");
                entries.remove(agent_id);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests;
