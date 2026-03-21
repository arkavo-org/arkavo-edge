//! Bridge between Iroh data plane and TDF encryption for inter-agent data sharing.
//!
//! Provides utility functions for the full cycle:
//! stage: data → TDF encrypt → Iroh blob → ticket string
//! fetch: ticket string → Iroh blob → TDF manifest bytes

#[cfg(feature = "iroh")]
use arkavo_tdf_iroh::IrohNode;

/// Stage data as an Iroh blob. Returns the ticket string for sharing via A2A.
///
/// For now, stages raw bytes without TDF encryption. TDF encryption can be
/// composed by the caller when KAS is configured.
#[cfg(feature = "iroh")]
pub async fn stage_blob(
    iroh_node: std::sync::Arc<IrohNode>,
    data: &[u8],
) -> Result<String, String> {
    use arkavo_tdf_iroh::IrohTransport;

    let transport = IrohTransport::new(iroh_node);
    let ticket = transport
        .stage_bytes(data)
        .await
        .map_err(|e| format!("Iroh stage failed: {e}"))?;

    let ticket_str = ticket.to_string();
    tracing::info!(
        bytes = data.len(),
        ticket_len = ticket_str.len(),
        "Staged blob on Iroh data plane"
    );
    Ok(ticket_str)
}

/// Fetch a blob from Iroh using a ticket string. Returns the raw bytes.
#[cfg(feature = "iroh")]
pub async fn fetch_blob(
    iroh_node: std::sync::Arc<IrohNode>,
    ticket_str: &str,
) -> Result<Vec<u8>, String> {
    use arkavo_tdf_iroh::{IrohTicket, IrohTransport};

    let ticket: IrohTicket = ticket_str
        .parse()
        .map_err(|e| format!("Invalid Iroh ticket: {e}"))?;

    let transport = IrohTransport::new(iroh_node);
    let data = transport
        .fetch_bytes(&ticket)
        .await
        .map_err(|e| format!("Iroh fetch failed: {e}"))?;

    tracing::info!(bytes = data.len(), "Fetched blob from Iroh data plane");
    Ok(data)
}
