//! Creator Profile Service Entry Point
//!
//! Binary for running the iroh.arkavo.net service.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use tokio::net::TcpListener;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use arkavo_profile_iroh::api::create_router;
use arkavo_profile_iroh::config::Config;
use arkavo_profile_iroh::profile::ProfileStore;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting arkavo-profile-iroh service...");

    // Load configuration
    let config = Config::from_env();
    info!("Configuration loaded:");
    info!("  Bind address: {}", config.bind_address);
    info!("  Port: {}", config.port);
    info!("  TLS enabled: {}", config.tls_enabled());
    info!("  Iroh data path: {:?}", config.iroh_data_path);

    // Initialize profile store
    let store = ProfileStore::new(&config).await?;
    let store = Arc::new(store);
    info!("Profile store initialized");

    // Create router
    let app = create_router(store.clone(), &config);

    // Bind address
    let addr: SocketAddr = format!("{}:{}", config.bind_address, config.port).parse()?;

    // Start server
    if config.tls_enabled() {
        start_tls_server(app, addr, &config).await
    } else {
        start_http_server(app, addr).await
    }
}

async fn start_http_server(app: Router, addr: SocketAddr) -> Result<()> {
    info!("Starting HTTP server on {}", addr);
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    info!("Server shutdown complete");
    Ok(())
}

async fn start_tls_server(app: Router, addr: SocketAddr, config: &Config) -> Result<()> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use rustls::ServerConfig;
    use rustls_pemfile::{certs, private_key};
    use std::fs::File;
    use std::io::BufReader;
    use tokio_rustls::TlsAcceptor;

    info!("Starting HTTPS server on {}", addr);

    // Load TLS certificate
    let cert_path = config
        .tls_cert_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("TLS cert path not configured"))?;
    let key_path = config
        .tls_key_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("TLS key path not configured"))?;

    let cert_file = File::open(cert_path)?;
    let key_file = File::open(key_path)?;

    let certs: Vec<CertificateDer<'static>> = certs(&mut BufReader::new(cert_file))
        .filter_map(Result::ok)
        .collect();

    let key: PrivateKeyDer<'static> = private_key(&mut BufReader::new(key_file))?
        .ok_or_else(|| anyhow::anyhow!("No private key found in {}", key_path.display()))?;

    let tls_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    let acceptor = TlsAcceptor::from(Arc::new(tls_config));
    let listener = TcpListener::bind(addr).await?;

    info!("TLS server listening on {}", addr);

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, peer_addr) = result?;
                let acceptor = acceptor.clone();
                let app = app.clone();

                tokio::spawn(async move {
                    match acceptor.accept(stream).await {
                        Ok(tls_stream) => {
                            let io = hyper_util::rt::TokioIo::new(tls_stream);
                            let service = tower::ServiceExt::into_make_service(app);
                            if let Err(e) = hyper::server::conn::http1::Builder::new()
                                .serve_connection(io, service)
                                .await
                            {
                                tracing::error!("Error serving connection from {}: {}", peer_addr, e);
                            }
                        }
                        Err(e) => {
                            tracing::error!("TLS handshake failed for {}: {}", peer_addr, e);
                        }
                    }
                });
            }
            _ = shutdown_signal() => {
                info!("Shutdown signal received");
                break;
            }
        }
    }

    info!("Server shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
