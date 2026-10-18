//! The images a task's agent delivers, fetched for the `arkavo ui`
//! workspace (AGUI-025).
//!
//! The gateway shows a task's images but cannot fetch them itself: that
//! takes the data plane and the manifest checks `arkavo chat` uses
//! (CLI-018). This hands the gateway the same path, writing under the
//! user's data directory rather than wherever `arkavo ui` was started.

use std::path::PathBuf;

use arkavo_agui::gateway_assets::{AssetFetcher, FetchedAsset};
use arkavo_protocol::types::MessagePart;
use arkavo_server::server::assets::{deliveries_in, take};
use arkavo_tdf_iroh::{FetchPolicy, IrohNode, IrohTransport};
use tokio::sync::OnceCell;

/// Fetches into `<data dir>/arkavo/ui-assets/<task id>`, over one data
/// plane node started at the first image and kept for the next.
pub(super) struct WorkspaceAssets {
    dir: PathBuf,
    transport: OnceCell<IrohTransport>,
}

impl WorkspaceAssets {
    pub(super) fn new() -> Self {
        let dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("arkavo")
            .join("ui-assets");
        Self {
            dir,
            transport: OnceCell::new(),
        }
    }

    async fn transport(&self) -> Result<&IrohTransport, String> {
        self.transport
            .get_or_try_init(|| async {
                IrohNode::memory()
                    .await
                    .map(IrohTransport::new)
                    .map_err(|e| format!("could not start the data plane to fetch images: {e}"))
            })
            .await
    }
}

#[async_trait::async_trait]
impl AssetFetcher for WorkspaceAssets {
    async fn fetch(
        &self,
        task_id: &str,
        parts: &[MessagePart],
    ) -> Result<Vec<FetchedAsset>, String> {
        let deliveries = deliveries_in(parts);
        let dir = super::assets::task_dir(&self.dir, task_id);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("cannot write {}: {e}", dir.display()))?;
        let transport = self.transport().await?;
        let mut fetched = Vec::new();
        let mut failed = Vec::new();
        for delivery in &deliveries {
            let taken = take(transport, delivery, &dir, FetchPolicy::default()).await;
            let license = delivery.manifest.license_line();
            for (entry, path) in taken.saved {
                fetched.push(FetchedAsset {
                    name: entry.name.clone(),
                    path,
                    width: entry.width,
                    height: entry.height,
                    has_alpha: entry.has_alpha,
                    bytes: entry.bytes,
                    license: license.clone(),
                });
            }
            failed.extend(
                taken
                    .failed
                    .into_iter()
                    .map(|(name, why)| format!("{name}: {why}")),
            );
        }
        if fetched.is_empty() && !failed.is_empty() {
            return Err(failed.join("; "));
        }
        Ok(fetched)
    }
}
