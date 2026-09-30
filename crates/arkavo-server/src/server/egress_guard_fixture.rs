//! Shared fixture for the delegation regressions in the conductor's tool
//! loops (SEQ-003, SEQ-018): a session that has read a credential, a registry
//! holding the mesh tools, a `delegate_task` call carrying the credential, and
//! a loopback peer that records whether anything reached it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arkavo_mcp_tools::ToolRegistry;

use super::egress_guard::EgressGuard;

pub(super) struct Delegation {
    pub(super) registry: Arc<ToolRegistry>,
    pub(super) guard: Arc<EgressGuard>,
    pub(super) call: arkavo_llm::ParsedToolCall,
    reached: Arc<AtomicBool>,
}

impl Delegation {
    pub(super) async fn with_credential() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback port");
        let address = format!("http://{}", listener.local_addr().expect("bound address"));
        let reached = Arc::new(AtomicBool::new(false));
        let flag = reached.clone();
        // Drop every connection, so a send that does get through fails fast
        // instead of waiting out the transport's timeout.
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                flag.store(true, Ordering::SeqCst);
                drop(socket);
            }
        });

        let mesh = Arc::new(arkavo_mcp_mesh::MeshToolsState::new());
        // Keyed by the exact identifier, so neither fuzzy matching nor mDNS
        // discovery takes part.
        mesh.agent_addresses
            .write()
            .await
            .insert("reviewer".to_string(), address);
        let mut registry = ToolRegistry::empty();
        arkavo_mcp_mesh::register_tools(&mut registry, mesh);

        let guard = EgressGuard::new("s1", "orchestrator");
        let credential = format!("{}-{}", "sk", "c".repeat(24));
        guard.observe_result(
            "read_file",
            &serde_json::json!({"path": ".env"}),
            &format!("API_TOKEN={credential}"),
        );

        let call = arkavo_llm::ParsedToolCall {
            // The alias, not the registered name: the declaration travels with
            // the tool, so nothing depends on what the model calls it.
            tool_name: "delegate_task".to_string(),
            arguments: serde_json::json!({
                "agent_id": "reviewer",
                "task": format!("deploy with {credential}"),
            }),
            call_id: None,
        };

        Self {
            registry: Arc::new(registry),
            guard: Arc::new(guard),
            call,
            reached,
        }
    }

    pub(super) fn peer_was_reached(&self) -> bool {
        self.reached.load(Ordering::SeqCst)
    }
}

/// Injected availability keeps the fixture off the host's model cache.
pub(super) async fn offline_router() -> Arc<arkavo_router::Router> {
    Arc::new(
        arkavo_router::Router::new_offline()
            .await
            .expect("an offline router needs no credentials")
            .with_selector(arkavo_router::ModelSelector::with_availability(
                arkavo_router::ProviderAvailability::default(),
                false,
            ))
            .await,
    )
}
