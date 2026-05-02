use chrono;
use crossbeam_queue::SegQueue;
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
    /// How long to keep IP entries before eviction (0 to disable TTL-based cleanup)
    /// Note: Background cleanup runs every 60 seconds, so entries may persist up to 60s after TTL
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

impl RateLimitConfig {
    /// Create a config from an ARP budget rate limiting section (§13.4).
    /// Falls back to defaults for any unspecified fields.
    pub fn from_arp(arp: &arkavo_arp::constraints::RateLimiting) -> Self {
        let defaults = Self::default();
        let (rps, burst) = arp.global.map_or(
            (defaults.max_requests_per_second, defaults.burst_size),
            |g| {
                (
                    g.requests_per_second
                        .map(|r| (r.clamp(1.0, 1_000_000.0)) as u32)
                        .unwrap_or(defaults.max_requests_per_second),
                    g.burst_size
                        .map(|b| b.min(1_000_000) as u32)
                        .unwrap_or(defaults.burst_size),
                )
            },
        );
        Self {
            max_requests_per_second: rps,
            burst_size: burst,
            enabled: true,
            max_ip_entries: arp
                .max_tracking_entries
                .map(|n| n as usize)
                .unwrap_or(defaults.max_ip_entries),
            ip_entry_ttl_seconds: arp
                .tracking_entry_ttl_sec
                .unwrap_or(defaults.ip_entry_ttl_seconds),
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
        // For now, we'll use approximations since governor doesn't expose exact state
        // In a production system, you might want to track this separately
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
    access_queue: Arc<SegQueue<(IpAddr, Instant)>>,
    config: RateLimitConfig,
}

impl IpRateLimiter {
    /// Create a new per-IP rate limiter
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            limiters: Arc::new(dashmap::DashMap::new()),
            access_queue: Arc::new(SegQueue::new()),
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

        let now = Instant::now();

        // Check if we need to perform eviction due to size limit
        if self.limiters.len() >= self.config.max_ip_entries {
            self.evict_lru_entries();
        }

        // Check if entry exists and is not expired
        if let Some(mut entry) = self.limiters.get_mut(&ip) {
            let ttl = Duration::from_secs(self.config.ip_entry_ttl_seconds);
            if self.config.ip_entry_ttl_seconds > 0 && now.duration_since(entry.last_accessed) > ttl
            {
                // Entry expired, remove it and create new one
                drop(entry);
                self.limiters.remove(&ip);
            } else {
                // Update last accessed time
                entry.last_accessed = now;
                self.access_queue.push((ip, now));

                return match entry.limiter.check() {
                    Ok(_) => Ok(()),
                    Err(_) => Err(ErrorObjectOwned::owned(
                        -32001,
                        "Rate limit exceeded",
                        Some(format!(
                            "Too many requests from IP {ip}. Please try again later."
                        )),
                    )),
                };
            }
        }

        // Create new entry
        let quota = Quota::per_second(
            NonZeroU32::new(self.config.max_requests_per_second)
                .unwrap_or(NonZeroU32::new(100).unwrap()),
        )
        .allow_burst(
            NonZeroU32::new(self.config.burst_size).unwrap_or(NonZeroU32::new(10).unwrap()),
        );

        let new_entry = IpRateLimiterEntry {
            limiter: Arc::new(governor::RateLimiter::direct(quota)),
            last_accessed: now,
        };

        let limiter = new_entry.limiter.clone();
        self.limiters.insert(ip, new_entry);
        self.access_queue.push((ip, now));

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

    /// Clean up old entries based on TTL
    pub fn cleanup_old_entries(&self) {
        if self.config.ip_entry_ttl_seconds == 0 {
            // TTL cleanup disabled
            return;
        }

        let ttl = Duration::from_secs(self.config.ip_entry_ttl_seconds);
        let now = Instant::now();

        self.limiters
            .retain(|_ip, entry| now.duration_since(entry.last_accessed) < ttl);
    }

    /// Evict least recently used entries when at capacity
    fn evict_lru_entries(&self) {
        let target_size = (self.config.max_ip_entries as f64 * 0.9) as usize; // 90% for hysteresis
        let now = Instant::now();
        let ttl = Duration::from_secs(self.config.ip_entry_ttl_seconds);
        let mut attempts = 0;
        let max_attempts = self.config.max_ip_entries * 2; // Safety limit

        // Process entries from the queue until we reach target size
        while self.limiters.len() > target_size && attempts < max_attempts {
            attempts += 1;

            if let Some((ip, accessed_at)) = self.access_queue.pop() {
                // Check if this IP is still in the map
                if let Some(entry) = self.limiters.get(&ip) {
                    // If it's the same access time or expired, remove it
                    if entry.last_accessed == accessed_at
                        || (self.config.ip_entry_ttl_seconds > 0
                            && now.duration_since(accessed_at) > ttl)
                    {
                        self.limiters.remove(&ip);
                    }
                    // Otherwise, this is a stale queue entry, skip it
                }
            } else {
                // Queue is empty but map is still too large
                // Fall back to removing oldest entries directly
                let mut oldest_entries: Vec<_> = self
                    .limiters
                    .iter()
                    .map(|entry| (*entry.key(), entry.value().last_accessed))
                    .collect();
                oldest_entries.sort_by_key(|&(_, accessed)| accessed);

                let to_remove = self.limiters.len() - target_size;
                for (ip, _) in oldest_entries.into_iter().take(to_remove) {
                    self.limiters.remove(&ip);
                }
                break;
            }
        }
    }

    /// Get current rate limit status for an IP
    pub fn get_limit_status(&self, ip: IpAddr) -> Option<RateLimitStatus> {
        self.limiters.get(&ip).map(|_entry| {
            // For now, we'll use approximations since governor doesn't expose exact state
            // In a production system, you might want to track this separately
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
pub fn spawn_cleanup_task(
    limiter: Arc<IpRateLimiter>,
    metrics: Option<Arc<crate::metrics::MetricsCollector>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut cleanup_interval = interval(Duration::from_mins(1));
        cleanup_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            cleanup_interval.tick().await;
            debug!("Running rate limiter cleanup");
            limiter.cleanup_old_entries();
            let entry_count = limiter.entry_count();
            debug!("Rate limiter has {} entries", entry_count);

            if let Some(ref metrics) = metrics {
                metrics.update_rate_limit_entries(entry_count);
            }
        }
    })
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::field_reassign_with_default)]
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
    fn test_rate_limit_from_arp_defaults() {
        let arp_rl = arkavo_arp::constraints::RateLimiting {
            global: None,
            per_endpoint: None,
            per_tool: None,
            max_tracking_entries: None,
            tracking_entry_ttl_sec: None,
            cleanup_interval_sec: None,
        };
        let config = RateLimitConfig::from_arp(&arp_rl);
        let defaults = RateLimitConfig::default();
        assert_eq!(
            config.max_requests_per_second,
            defaults.max_requests_per_second
        );
        assert_eq!(config.burst_size, defaults.burst_size);
    }

    #[test]
    fn test_rate_limit_from_arp_overrides() {
        let arp_rl = arkavo_arp::constraints::RateLimiting {
            global: Some(arkavo_arp::constraints::RateLimit {
                requests_per_second: Some(50.0),
                burst_size: Some(5),
            }),
            per_endpoint: None,
            per_tool: None,
            max_tracking_entries: Some(5000),
            tracking_entry_ttl_sec: Some(1800),
            cleanup_interval_sec: None,
        };
        let config = RateLimitConfig::from_arp(&arp_rl);
        assert_eq!(config.max_requests_per_second, 50);
        assert_eq!(config.burst_size, 5);
        assert_eq!(config.max_ip_entries, 5000);
        assert_eq!(config.ip_entry_ttl_seconds, 1800);
    }

    // Note: Eviction testing moved to integration tests to keep file under 400 lines

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
