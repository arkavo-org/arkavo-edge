use arkavo_protocol::metrics::{MetricsCollector, RpcTimer};
use arkavo_protocol::rate_limit::RateLimiter;
use arkavo_trust::{
    SharedTrustService, TrustHistoryRequest, TrustHistoryResponse, TrustProvidersResponse,
    TrustPublishRequest, TrustPublishResponse, TrustQueryRequest, TrustQueryResponse,
    TrustVerifyRequest, TrustVerifyResponse,
};
use jsonrpsee::types::ErrorObjectOwned;
use std::sync::Arc;

#[allow(clippy::unused_async)]
pub async fn handle_trust_query(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    trust_service: &SharedTrustService,
    request: TrustQueryRequest,
) -> Result<TrustQueryResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("trust_query".to_string(), metrics.clone());

    if let Err(_e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(ErrorObjectOwned::owned(
            arkavo_trust::error_codes::RATE_LIMITED,
            "Rate limit exceeded",
            None::<()>,
        ));
    }

    match trust_service.query(&request) {
        Ok(response) => {
            timer.success();
            Ok(response)
        }
        Err(e) => {
            timer.error();
            Err(ErrorObjectOwned::owned(
                e.error_code(),
                e.to_string(),
                None::<()>,
            ))
        }
    }
}

#[allow(clippy::unused_async)]
pub async fn handle_trust_verify(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    trust_service: &SharedTrustService,
    request: TrustVerifyRequest,
) -> Result<TrustVerifyResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("trust_verify".to_string(), metrics.clone());

    if let Err(_e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(ErrorObjectOwned::owned(
            arkavo_trust::error_codes::RATE_LIMITED,
            "Rate limit exceeded",
            None::<()>,
        ));
    }

    match trust_service.verify(&request) {
        Ok(response) => {
            timer.success();
            Ok(response)
        }
        Err(e) => {
            timer.error();
            Err(ErrorObjectOwned::owned(
                e.error_code(),
                e.to_string(),
                None::<()>,
            ))
        }
    }
}

#[allow(clippy::unused_async)]
pub async fn handle_trust_history(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    trust_service: &SharedTrustService,
    request: TrustHistoryRequest,
) -> Result<TrustHistoryResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("trust_history".to_string(), metrics.clone());

    if let Err(_e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(ErrorObjectOwned::owned(
            arkavo_trust::error_codes::RATE_LIMITED,
            "Rate limit exceeded",
            None::<()>,
        ));
    }

    let response = trust_service.history(&request);
    timer.success();
    Ok(response)
}

#[allow(clippy::unused_async)]
pub async fn handle_trust_publish(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    trust_service: &SharedTrustService,
    request: TrustPublishRequest,
) -> Result<TrustPublishResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("trust_publish".to_string(), metrics.clone());

    if let Err(_e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(ErrorObjectOwned::owned(
            arkavo_trust::error_codes::RATE_LIMITED,
            "Rate limit exceeded",
            None::<()>,
        ));
    }

    match trust_service.publish(&request) {
        Ok(response) => {
            timer.success();
            Ok(response)
        }
        Err(e) => {
            timer.error();
            Err(ErrorObjectOwned::owned(
                e.error_code(),
                e.to_string(),
                None::<()>,
            ))
        }
    }
}

#[allow(clippy::unused_async)]
pub async fn handle_trust_providers(
    metrics: &Arc<MetricsCollector>,
    trust_service: &SharedTrustService,
) -> Result<TrustProvidersResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("trust_providers".to_string(), metrics.clone());
    let response = trust_service.providers();
    timer.success();
    Ok(response)
}
