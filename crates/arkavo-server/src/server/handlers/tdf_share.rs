//! TDF P2P share RPC handlers for interagent encrypted data exchange.

use arkavo_protocol::metrics::{MetricsCollector, RpcTimer};
use arkavo_protocol::rate_limit::RateLimiter;
use arkavo_protocol::types::{
    TdfOffer, TdfOffersRequest, TdfOffersResponse, TdfShareRequest, TdfShareResponse,
};
use chrono::Utc;
use jsonrpsee::types::ErrorObjectOwned;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;

const MAX_OFFERS: usize = 1000;
const OFFER_TTL: Duration = Duration::from_hours(1);

/// In-memory store for pending TDF share offers with capacity and TTL limits.
#[derive(Debug)]
pub struct TdfOfferStore {
    offers: RwLock<Vec<(Instant, TdfOffer)>>,
}

impl Default for TdfOfferStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TdfOfferStore {
    pub fn new() -> Self {
        Self {
            offers: RwLock::new(Vec::new()),
        }
    }

    pub async fn add(&self, offer: TdfOffer) {
        let mut offers = self.offers.write().await;
        // Evict expired offers
        let now = Instant::now();
        offers.retain(|(created, _)| now.duration_since(*created) < OFFER_TTL);
        // Evict oldest if at capacity
        if offers.len() >= MAX_OFFERS {
            offers.remove(0);
        }
        offers.push((now, offer));
    }

    pub async fn list(&self) -> Vec<TdfOffer> {
        let mut offers = self.offers.write().await;
        let now = Instant::now();
        offers.retain(|(created, _)| now.duration_since(*created) < OFFER_TTL);
        offers.iter().map(|(_, o)| o.clone()).collect()
    }
}

/// Handle tdf.share RPC method.
///
/// Stores an incoming TDF share offer from a remote agent.
pub async fn handle_tdf_share(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    offer_store: &Arc<TdfOfferStore>,
    request: TdfShareRequest,
) -> Result<TdfShareResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("tdf.share".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    // Validate field lengths to prevent memory exhaustion and injection
    if request.ticket.is_empty() {
        timer.error();
        return Err(ErrorObjectOwned::owned(
            -32602,
            "Invalid params",
            Some("ticket must not be empty".to_string()),
        ));
    }
    if request.content_hash.len() > 256 {
        timer.error();
        return Err(ErrorObjectOwned::owned(
            -32602,
            "Invalid params",
            Some("content_hash exceeds 256 chars".to_string()),
        ));
    }
    if request.sender_agent_id.len() > 256 {
        timer.error();
        return Err(ErrorObjectOwned::owned(
            -32602,
            "Invalid params",
            Some("sender_agent_id exceeds 256 chars".to_string()),
        ));
    }
    if request.kas_url.len() > 2048 {
        timer.error();
        return Err(ErrorObjectOwned::owned(
            -32602,
            "Invalid params",
            Some("kas_url exceeds 2048 chars".to_string()),
        ));
    }
    if request.policy_attributes.len() > 50 {
        timer.error();
        return Err(ErrorObjectOwned::owned(
            -32602,
            "Invalid params",
            Some("policy_attributes exceeds 50 entries".to_string()),
        ));
    }
    if request.size_bytes > 10_737_418_240 {
        timer.error();
        return Err(ErrorObjectOwned::owned(
            -32602,
            "Invalid params",
            Some("size_bytes exceeds 10 GB limit".to_string()),
        ));
    }

    let offer_id = Uuid::new_v4().to_string();

    let offer = TdfOffer {
        offer_id: offer_id.clone(),
        ticket: request.ticket,
        content_hash: request.content_hash,
        size_bytes: request.size_bytes,
        policy_attributes: request.policy_attributes,
        kas_url: request.kas_url,
        sender_agent_id: request.sender_agent_id,
        received_at: Utc::now().to_rfc3339(),
    };

    offer_store.add(offer).await;

    timer.success();
    Ok(TdfShareResponse {
        accepted: true,
        offer_id,
    })
}

/// Handle tdf.offers RPC method.
///
/// Returns pending TDF share offers for this agent.
pub async fn handle_tdf_offers(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    offer_store: &Arc<TdfOfferStore>,
    _request: TdfOffersRequest,
) -> Result<TdfOffersResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("tdf.offers".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    let offers = offer_store.list().await;

    timer.success();
    Ok(TdfOffersResponse { offers })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_protocol::rate_limit::RateLimitConfig;

    fn create_test_metrics() -> Arc<MetricsCollector> {
        Arc::new(MetricsCollector::new(false))
    }

    fn create_test_rate_limiter() -> RateLimiter {
        RateLimiter::new(RateLimitConfig::default())
    }

    fn create_test_request() -> TdfShareRequest {
        TdfShareRequest {
            ticket: "blobaaaa1234567890".to_string(),
            content_hash: "abc123def456".to_string(),
            size_bytes: 4096,
            policy_attributes: vec!["https://arkavo.net/attr/sensitivity".to_string()],
            kas_url: "https://kas.arkavo.net".to_string(),
            sender_agent_id: "agent-sender-001".to_string(),
        }
    }

    #[tokio::test]
    async fn test_tdf_share_stores_offer() {
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();
        let store = Arc::new(TdfOfferStore::new());

        let request = create_test_request();
        let result = handle_tdf_share(&metrics, &rate_limiter, &store, request).await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.accepted);
        assert!(!response.offer_id.is_empty());

        // Verify stored
        let offers = store.list().await;
        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].sender_agent_id, "agent-sender-001");
    }

    #[tokio::test]
    async fn test_tdf_share_empty_ticket_rejected() {
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();
        let store = Arc::new(TdfOfferStore::new());

        let mut request = create_test_request();
        request.ticket = String::new();

        let result = handle_tdf_share(&metrics, &rate_limiter, &store, request).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), -32602);
    }

    #[tokio::test]
    async fn test_tdf_offers_empty() {
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();
        let store = Arc::new(TdfOfferStore::new());

        let result = handle_tdf_offers(&metrics, &rate_limiter, &store, TdfOffersRequest {}).await;

        assert!(result.is_ok());
        assert!(result.unwrap().offers.is_empty());
    }

    #[tokio::test]
    async fn test_tdf_share_rejects_oversized_content_hash() {
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();
        let store = Arc::new(TdfOfferStore::new());

        let mut request = create_test_request();
        request.content_hash = "x".repeat(257);

        let result = handle_tdf_share(&metrics, &rate_limiter, &store, request).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), -32602);
    }

    #[tokio::test]
    async fn test_tdf_share_rejects_oversized_sender_id() {
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();
        let store = Arc::new(TdfOfferStore::new());

        let mut request = create_test_request();
        request.sender_agent_id = "x".repeat(257);

        let result = handle_tdf_share(&metrics, &rate_limiter, &store, request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_tdf_share_rejects_oversized_kas_url() {
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();
        let store = Arc::new(TdfOfferStore::new());

        let mut request = create_test_request();
        request.kas_url = "x".repeat(2049);

        let result = handle_tdf_share(&metrics, &rate_limiter, &store, request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_tdf_share_rejects_too_many_policy_attributes() {
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();
        let store = Arc::new(TdfOfferStore::new());

        let mut request = create_test_request();
        request.policy_attributes = (0..51).map(|i| format!("attr-{i}")).collect();

        let result = handle_tdf_share(&metrics, &rate_limiter, &store, request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_tdf_share_rejects_oversized_bytes() {
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();
        let store = Arc::new(TdfOfferStore::new());

        let mut request = create_test_request();
        request.size_bytes = 10_737_418_241;

        let result = handle_tdf_share(&metrics, &rate_limiter, &store, request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_tdf_offers_returns_stored() {
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();
        let store = Arc::new(TdfOfferStore::new());

        // Share two offers
        let req1 = create_test_request();
        let mut req2 = create_test_request();
        req2.sender_agent_id = "agent-sender-002".to_string();

        handle_tdf_share(&metrics, &rate_limiter, &store, req1)
            .await
            .unwrap();
        handle_tdf_share(&metrics, &rate_limiter, &store, req2)
            .await
            .unwrap();

        let result = handle_tdf_offers(&metrics, &rate_limiter, &store, TdfOffersRequest {}).await;

        assert!(result.is_ok());
        let offers = result.unwrap().offers;
        assert_eq!(offers.len(), 2);
    }
}
