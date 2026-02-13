use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode},
    middleware::Next,
    response::Response,
};
use ipnet::IpNet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tracing::warn;

use crate::rate_limit::IpRateLimiter;

/// Extract the real client IP, honoring proxy headers only from trusted sources.
pub fn extract_client_ip(
    headers: &HeaderMap,
    socket_addr: SocketAddr,
    trusted_proxies: &[IpNet],
) -> IpAddr {
    if trusted_proxies
        .iter()
        .any(|net| net.contains(&socket_addr.ip()))
    {
        if let Some(xff) = headers.get("x-forwarded-for")
            && let Some(first_ip) = xff
                .to_str()
                .ok()
                .and_then(|s| s.split(',').next())
                .and_then(|s| s.trim().parse::<IpAddr>().ok())
        {
            return first_ip;
        }
        if let Some(real_ip) = headers.get("x-real-ip")
            && let Ok(ip) = real_ip.to_str().unwrap_or("").parse::<IpAddr>()
        {
            return ip;
        }
    }
    socket_addr.ip()
}

/// Axum middleware that applies per-IP rate limiting.
pub async fn ip_rate_limit_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let rate_limiter = request.extensions().get::<Arc<IpRateLimiter>>().cloned();

    let trusted_proxies = request
        .extensions()
        .get::<Arc<Vec<IpNet>>>()
        .cloned()
        .unwrap_or_default();

    if let Some(limiter) = rate_limiter {
        let client_ip = extract_client_ip(request.headers(), addr, &trusted_proxies);
        if limiter.check_rate_limit(client_ip).is_err() {
            warn!(
                event = "rate_limit",
                action = "deny",
                client_ip = %client_ip,
                "Rate limit exceeded"
            );
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
    }

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_client_ip_direct() {
        let headers = HeaderMap::new();
        let addr: SocketAddr = "10.0.0.1:1234".parse().unwrap();
        let ip = extract_client_ip(&headers, addr, &[]);
        assert_eq!(ip, "10.0.0.1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn test_extract_client_ip_xff_untrusted() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "1.2.3.4".parse().unwrap());
        let addr: SocketAddr = "10.0.0.1:1234".parse().unwrap();
        // Not from trusted proxy, so XFF is ignored
        let ip = extract_client_ip(&headers, addr, &[]);
        assert_eq!(ip, "10.0.0.1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn test_extract_client_ip_xff_trusted() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "1.2.3.4, 10.0.0.1".parse().unwrap());
        let addr: SocketAddr = "10.0.0.1:1234".parse().unwrap();
        let trusted: Vec<IpNet> = vec!["10.0.0.0/8".parse().unwrap()];
        let ip = extract_client_ip(&headers, addr, &trusted);
        assert_eq!(ip, "1.2.3.4".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn test_extract_client_ip_real_ip_trusted() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "5.6.7.8".parse().unwrap());
        let addr: SocketAddr = "172.16.0.1:1234".parse().unwrap();
        let trusted: Vec<IpNet> = vec!["172.16.0.0/12".parse().unwrap()];
        let ip = extract_client_ip(&headers, addr, &trusted);
        assert_eq!(ip, "5.6.7.8".parse::<IpAddr>().unwrap());
    }
}
