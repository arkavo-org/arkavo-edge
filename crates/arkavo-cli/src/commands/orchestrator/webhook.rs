use anyhow::Result;
use arkavo_orchestrator::{GitHubApp, OrchestratorConfig, WebhookServer, oidc::OidcProvider};
use arkavo_protocol::rate_limit::RateLimitConfig;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::signal;
use tracing::{error, info};

use super::constants::{IP_ENTRY_TTL_SECONDS, MAX_IP_ENTRIES};
use super::init::create_orchestrator;

pub(super) async fn start_orchestrator(
    port: u16,
    webhook_secret: Option<String>,
    app_id: Option<String>,
    private_key: Option<String>,
) -> Result<()> {
    info!("Starting GitHub orchestrator server");

    let config = if let (Some(secret), Some(id), Some(key)) = (webhook_secret, app_id, private_key)
    {
        OrchestratorConfig {
            webhook_secret: Some(secret),
            github_app_id: id,
            github_app_private_key: key,
            webhook_port: port,
            ..Default::default()
        }
    } else {
        OrchestratorConfig::from_env()?
    };

    info!("Webhook server will listen on port {}", config.webhook_port);
    info!("GitHub App ID: {}", config.github_app_id);
    info!("Webhook secret: {}", config.get_masked_secret());

    let webhook_secret = config.webhook_secret.ok_or_else(|| {
        anyhow::anyhow!(
            "Webhook secret is required for webhook server mode. \
            Set ARKAVO_GITHUB_WEBHOOK_SECRET environment variable or use polling mode instead."
        )
    })?;

    let rate_limit_config = RateLimitConfig {
        max_requests_per_second: config.rate_limit_requests_per_second,
        burst_size: config.rate_limit_burst_size,
        enabled: true,
        max_ip_entries: MAX_IP_ENTRIES,
        ip_entry_ttl_seconds: IP_ENTRY_TTL_SECONDS,
    };

    let (webhook_server, mut event_rx) = WebhookServer::new(webhook_secret, rate_limit_config);

    let github_app = Arc::new(GitHubApp::new(
        config.github_app_id.parse()?,
        &config.github_app_private_key,
    )?);

    let orchestrator = create_orchestrator(github_app).await?;

    // Create OIDC provider for local OpenTDF authentication
    let issuer = format!("http://localhost:{}", config.webhook_port);
    let oidc_provider = Arc::new(OidcProvider::new(issuer.clone()));
    info!("OIDC provider initialized at {}", issuer);

    // Create router with OIDC endpoints merged
    let app = webhook_server.router_with_oidc(oidc_provider);
    let addr = SocketAddr::from(([0, 0, 0, 0], config.webhook_port));

    info!("Starting webhook server on {}", addr);

    let orchestrator_clone = Arc::clone(&orchestrator);
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            if let arkavo_orchestrator::GitHubEvent::Issues(issue_event) = event {
                info!(
                    issue = issue_event.issue.number,
                    repository = %issue_event.repository.full_name,
                    "Received issue event"
                );

                if let Err(e) = orchestrator_clone.handle_issue_event(*issue_event).await {
                    error!(error = %e, "Failed to handle issue event");
                }
            }
        }
    });

    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("Webhook server listening on {}", addr);
    info!("Orchestrator is ready to process GitHub events");

    let shutdown_signal = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install CTRL+C signal handler");
        info!("Shutdown signal received");
    };

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal)
    .await?;

    orchestrator.stop().await?;
    info!("Orchestrator stopped");

    Ok(())
}
