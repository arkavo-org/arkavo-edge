use arkavo_agent::registration::{
    ChallengeRequest, ChallengeResponse, RegistrationService, RegistrationStatus, VerifyRequest,
    VerifyResponse,
};
use arkavo_protocol::metrics::{MetricsCollector, RpcTimer};
use arkavo_protocol::rate_limit::RateLimiter;
use jsonrpsee::types::ErrorObjectOwned;
use std::sync::Arc;

pub async fn handle_registration_challenge(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    registration_service: &Arc<RegistrationService>,
    request: ChallengeRequest,
) -> Result<ChallengeResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("registration_challenge".to_string(), metrics.clone());

    if let Err(_e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(ErrorObjectOwned::owned(
            429,
            "Rate limit exceeded",
            None::<()>,
        ));
    }

    match registration_service.create_challenge(request).await {
        Ok(response) => {
            timer.success();
            Ok(response)
        }
        Err(e) => {
            timer.error();
            Err(ErrorObjectOwned::owned(
                -32000,
                format!("Registration error: {e}"),
                None::<()>,
            ))
        }
    }
}

pub async fn handle_registration_verify(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    registration_service: &Arc<RegistrationService>,
    request: VerifyRequest,
) -> Result<VerifyResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("registration_verify".to_string(), metrics.clone());

    if let Err(_e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(ErrorObjectOwned::owned(
            429,
            "Rate limit exceeded",
            None::<()>,
        ));
    }

    match registration_service.verify_challenge(request).await {
        Ok(response) => {
            timer.success();
            Ok(response)
        }
        Err(e) => {
            timer.error();
            Err(ErrorObjectOwned::owned(
                -32000,
                format!("Verification error: {e}"),
                None::<()>,
            ))
        }
    }
}

pub async fn handle_registration_status(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    registration_service: &Arc<RegistrationService>,
    device_id: String,
) -> Result<RegistrationStatus, ErrorObjectOwned> {
    let timer = RpcTimer::new("registration_status".to_string(), metrics.clone());

    if let Err(_e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(ErrorObjectOwned::owned(
            429,
            "Rate limit exceeded",
            None::<()>,
        ));
    }

    match registration_service
        .get_registration_status(&device_id)
        .await
    {
        Ok(status) => {
            timer.success();
            Ok(status)
        }
        Err(e) => {
            timer.error();
            Err(ErrorObjectOwned::owned(
                -32000,
                format!("Status error: {e}"),
                None::<()>,
            ))
        }
    }
}
