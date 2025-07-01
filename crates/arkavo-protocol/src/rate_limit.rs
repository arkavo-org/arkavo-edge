use governor::{Jitter, Quota};
use jsonrpsee::types::ErrorObjectOwned;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

/// Configuration for rate limiting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Maximum requests per second
    pub max_requests_per_second: u32,
    /// Burst size (allows temporary spikes)
    pub burst_size: u32,
    /// Whether rate limiting is enabled
    pub enabled: bool,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests_per_second: 100,
            burst_size: 10,
            enabled: true,
        }
    }
}

/// Rate limiter for A2A protocol
pub struct RateLimiter {
    limiter: Arc<governor::DefaultDirectRateLimiter>,
    config: RateLimitConfig,
}

impl RateLimiter {
    /// Create a new rate limiter with the given configuration
    ///
    /// # Panics
    ///
    /// This should not panic as we always provide fallback values
    pub fn new(config: RateLimitConfig) -> Self {
        let quota = Quota::per_second(
            NonZeroU32::new(config.max_requests_per_second)
                .unwrap_or(NonZeroU32::new(100).unwrap()),
        )
        .allow_burst(NonZeroU32::new(config.burst_size).unwrap_or(NonZeroU32::new(10).unwrap()));

        let limiter = Arc::new(governor::RateLimiter::direct(quota));

        Self { limiter, config }
    }

    /// Check if a request should be allowed
    pub fn check_rate_limit(&self) -> Result<(), ErrorObjectOwned> {
        if !self.config.enabled {
            return Ok(());
        }

        match self.limiter.check() {
            Ok(_) => Ok(()),
            Err(_) => Err(ErrorObjectOwned::owned(
                -32001,
                "Rate limit exceeded",
                Some("Too many requests. Please try again later.".to_string()),
            )),
        }
    }

    /// Check rate limit with jitter (for retry scenarios)
    pub async fn check_rate_limit_with_jitter(&self) -> Result<(), ErrorObjectOwned> {
        if !self.config.enabled {
            return Ok(());
        }

        let jitter = Jitter::up_to(Duration::from_millis(100));
        match self.limiter.check() {
            Ok(_) => Ok(()),
            Err(not_until) => {
                let delay = jitter
                    + not_until.wait_time_from(governor::clock::Clock::now(
                        &governor::clock::QuantaClock::default(),
                    ));
                tokio::time::sleep(delay).await;
                Err(ErrorObjectOwned::owned(
                    -32001,
                    "Rate limit exceeded",
                    Some(format!(
                        "Too many requests. Retry after {} ms",
                        delay.as_millis()
                    )),
                ))
            }
        }
    }
}

/// Per-IP rate limiter for more granular control
pub struct IpRateLimiter {
    limiters: Arc<dashmap::DashMap<IpAddr, Arc<governor::DefaultDirectRateLimiter>>>,
    config: RateLimitConfig,
}

impl IpRateLimiter {
    /// Create a new per-IP rate limiter
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            limiters: Arc::new(dashmap::DashMap::new()),
            config,
        }
    }

    /// Check rate limit for a specific IP address
    ///
    /// # Panics
    ///
    /// This should not panic as we always provide fallback values
    pub fn check_rate_limit(&self, ip: IpAddr) -> Result<(), ErrorObjectOwned> {
        if !self.config.enabled {
            return Ok(());
        }

        let limiter = self.limiters.entry(ip).or_insert_with(|| {
            let quota = Quota::per_second(
                NonZeroU32::new(self.config.max_requests_per_second)
                    .unwrap_or(NonZeroU32::new(100).unwrap()),
            )
            .allow_burst(
                NonZeroU32::new(self.config.burst_size).unwrap_or(NonZeroU32::new(10).unwrap()),
            );
            Arc::new(governor::RateLimiter::direct(quota))
        });

        match limiter.check() {
            Ok(_) => Ok(()),
            Err(_) => Err(ErrorObjectOwned::owned(
                -32001,
                "Rate limit exceeded",
                Some(format!(
                    "Too many requests from IP {ip}. Please try again later."
                )),
            )),
        }
    }

    /// Clean up old entries (call periodically to prevent memory leaks)
    pub fn cleanup_old_entries(&self, _max_age: Duration) {
        // In a real implementation, we'd track last access time
        // For now, we'll clear entries that haven't been used recently
        // This is a simplified version
        if self.limiters.len() > 10000 {
            self.limiters.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limit_config_default() {
        let config = RateLimitConfig::default();
        assert_eq!(config.max_requests_per_second, 100);
        assert_eq!(config.burst_size, 10);
        assert!(config.enabled);
    }

    #[test]
    fn test_rate_limiter_creation() {
        let config = RateLimitConfig::default();
        let _limiter = RateLimiter::new(config);
    }

    #[test]
    fn test_rate_limiter_disabled() {
        let mut config = RateLimitConfig::default();
        config.enabled = false;
        let limiter = RateLimiter::new(config);

        // Should always allow when disabled
        assert!(limiter.check_rate_limit().is_ok());
    }

    #[test]
    fn test_rate_limiter_basic() {
        let mut config = RateLimitConfig::default();
        config.max_requests_per_second = 1;
        config.burst_size = 1;
        let limiter = RateLimiter::new(config);

        // First request should succeed
        assert!(limiter.check_rate_limit().is_ok());

        // Second immediate request should fail
        let result = limiter.check_rate_limit();
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.code(), -32001);
            assert!(e.message().contains("Rate limit"));
        }
    }

    #[test]
    fn test_ip_rate_limiter() {
        let config = RateLimitConfig {
            max_requests_per_second: 1,
            burst_size: 1,
            enabled: true,
        };
        let limiter = IpRateLimiter::new(config);

        let ip1: IpAddr = "127.0.0.1".parse().unwrap();
        let ip2: IpAddr = "192.168.1.1".parse().unwrap();

        // First request from IP1 should succeed
        assert!(limiter.check_rate_limit(ip1).is_ok());

        // Second request from IP1 should fail
        assert!(limiter.check_rate_limit(ip1).is_err());

        // First request from IP2 should succeed (different IP)
        assert!(limiter.check_rate_limit(ip2).is_ok());
    }
}
