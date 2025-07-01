use chrono;
use governor::{Jitter, Quota};
use jsonrpsee::types::ErrorObjectOwned;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::interval;
use tracing::debug;

/// Configuration for rate limiting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Maximum requests per second
    pub max_requests_per_second: u32,
    /// Burst size (allows temporary spikes)
    pub burst_size: u32,
    /// Whether rate limiting is enabled
    pub enabled: bool,
    /// Maximum number of IP entries to track (for per-IP limiting)
    pub max_ip_entries: usize,
    /// How long to keep IP entries before eviction
    pub ip_entry_ttl_seconds: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests_per_second: 100,
            burst_size: 10,
            enabled: true,
            max_ip_entries: 10_000,
            ip_entry_ttl_seconds: 3600, // 1 hour
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

    /// Get current rate limit status
    pub fn get_limit_status(&self) -> RateLimitStatus {
        // Since we can't access the internal state directly, we'll estimate
        // based on configuration. In production, you might want to track this separately.
        RateLimitStatus {
            limit: self.config.max_requests_per_second,
            remaining: self.config.burst_size, // This is an approximation
            reset_at: chrono::Utc::now() + chrono::Duration::seconds(1),
        }
    }
}

/// Entry in the IP rate limiter, tracking limiter and last access time
struct IpRateLimiterEntry {
    limiter: Arc<governor::DefaultDirectRateLimiter>,
    last_accessed: Instant,
}

/// Per-IP rate limiter for more granular control
pub struct IpRateLimiter {
    limiters: Arc<dashmap::DashMap<IpAddr, IpRateLimiterEntry>>,
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

        // Check if we need to perform eviction due to size limit
        if self.limiters.len() >= self.config.max_ip_entries {
            self.evict_lru_entries();
        }

        let mut entry = self.limiters.entry(ip).or_insert_with(|| {
            let quota = Quota::per_second(
                NonZeroU32::new(self.config.max_requests_per_second)
                    .unwrap_or(NonZeroU32::new(100).unwrap()),
            )
            .allow_burst(
                NonZeroU32::new(self.config.burst_size).unwrap_or(NonZeroU32::new(10).unwrap()),
            );
            IpRateLimiterEntry {
                limiter: Arc::new(governor::RateLimiter::direct(quota)),
                last_accessed: Instant::now(),
            }
        });

        // Update last accessed time
        entry.last_accessed = Instant::now();

        match entry.limiter.check() {
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

    /// Clean up old entries based on TTL
    pub fn cleanup_old_entries(&self) {
        let ttl = Duration::from_secs(self.config.ip_entry_ttl_seconds);
        let now = Instant::now();

        self.limiters
            .retain(|_ip, entry| now.duration_since(entry.last_accessed) < ttl);
    }

    /// Evict least recently used entries when at capacity
    fn evict_lru_entries(&self) {
        // Collect entries with their last access times
        let mut entries: Vec<(IpAddr, Instant)> = self
            .limiters
            .iter()
            .map(|entry| (*entry.key(), entry.value().last_accessed))
            .collect();

        // Sort by last accessed time (oldest first)
        entries.sort_by_key(|&(_, last_accessed)| last_accessed);

        // Remove the oldest 10% of entries
        let to_remove = self.config.max_ip_entries / 10;
        for (ip, _) in entries.into_iter().take(to_remove) {
            self.limiters.remove(&ip);
        }
    }

    /// Get current rate limit status for an IP
    pub fn get_limit_status(&self, ip: IpAddr) -> Option<RateLimitStatus> {
        self.limiters.get(&ip).map(|_entry| {
            // Since we can't access the internal state directly, we'll estimate
            // based on configuration. In production, you might want to track this separately.
            RateLimitStatus {
                limit: self.config.max_requests_per_second,
                remaining: self.config.burst_size, // This is an approximation
                reset_at: chrono::Utc::now() + chrono::Duration::seconds(1),
            }
        })
    }

    /// Get the current number of tracked IPs
    pub fn entry_count(&self) -> usize {
        self.limiters.len()
    }
}

/// Rate limit status information
#[derive(Debug, Clone)]
pub struct RateLimitStatus {
    /// Maximum requests per window
    pub limit: u32,
    /// Remaining requests in current window
    pub remaining: u32,
    /// When the window resets
    pub reset_at: chrono::DateTime<chrono::Utc>,
}

/// Spawn a background task to periodically clean up old IP rate limiter entries
pub fn spawn_cleanup_task(limiter: Arc<IpRateLimiter>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut cleanup_interval = interval(Duration::from_secs(60)); // Run every minute
        cleanup_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            cleanup_interval.tick().await;
            debug!("Running rate limiter cleanup");
            limiter.cleanup_old_entries();
            debug!("Rate limiter has {} entries", limiter.entry_count());
        }
    })
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
        assert_eq!(config.max_ip_entries, 10_000);
        assert_eq!(config.ip_entry_ttl_seconds, 3600);
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
            max_ip_entries: 10_000,
            ip_entry_ttl_seconds: 3600,
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

    #[test]
    fn test_ip_rate_limiter_eviction() {
        let config = RateLimitConfig {
            max_requests_per_second: 10,
            burst_size: 1,
            enabled: true,
            max_ip_entries: 10,
            ip_entry_ttl_seconds: 3600,
        };
        let limiter = IpRateLimiter::new(config);

        // Add entries up to the limit
        for i in 0..10 {
            let ip: IpAddr = format!("192.168.1.{}", i).parse().unwrap();
            let _ = limiter.check_rate_limit(ip);
        }

        assert_eq!(limiter.limiters.len(), 10);

        // Add one more, triggering eviction
        let ip: IpAddr = "192.168.1.10".parse().unwrap();
        let _ = limiter.check_rate_limit(ip);

        // Should have evicted some entries
        assert!(limiter.limiters.len() <= 10);
    }

    #[test]
    fn test_rate_limit_status() {
        let config = RateLimitConfig::default();
        let limiter = RateLimiter::new(config);

        // Get initial status
        let status = limiter.get_limit_status();
        assert_eq!(status.limit, 100);
        assert!(status.remaining <= 10); // burst size
        assert!(status.reset_at > chrono::Utc::now());
    }
}
