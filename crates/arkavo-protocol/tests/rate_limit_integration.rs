#![allow(clippy::disallowed_methods)]

use arkavo_protocol::{IpRateLimiter, RateLimitConfig, spawn_cleanup_task};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;

#[tokio::test]
async fn test_rate_limit_eviction_with_background_task() {
    let config = RateLimitConfig {
        max_requests_per_second: 10,
        burst_size: 1,
        enabled: true,
        max_ip_entries: 100,
        ip_entry_ttl_seconds: 2, // Short TTL for testing
    };

    let limiter = Arc::new(IpRateLimiter::new(config));

    // Spawn cleanup task (no metrics for test)
    let cleanup_handle = spawn_cleanup_task(limiter.clone(), None);

    // Add some IPs
    for i in 0..50 {
        let ip: IpAddr = format!("192.168.1.{i}").parse().unwrap();
        let _ = limiter.check_rate_limit(ip);
    }

    assert_eq!(limiter.entry_count(), 50);

    // Wait for TTL to expire
    time::sleep(Duration::from_secs(3)).await;

    // Manually trigger cleanup since the background task runs every minute
    limiter.cleanup_old_entries();

    // All old entries should be cleaned up except the ones we just added
    assert_eq!(limiter.entry_count(), 0); // All entries should be expired

    // Cleanup
    cleanup_handle.abort();
}

/// Regression test for the eviction path in `evict_lru_entries`.
///
/// Inserting far more distinct IPs than `max_ip_entries` forces repeated
/// eviction where the popped queue entry's IP is still live with a matching
/// access time. Previously this held a dashmap read `Ref` across a `remove` on
/// the same key, which self-deadlocked the shard and hung the nightly fuzzer.
/// It also exhausted the attempt budget on stale entries without the fallback
/// sweep, letting the map grow past capacity. This test would hang (or trip the
/// capacity assertion) before the fix and must finish quickly after it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_eviction_stays_under_capacity_under_churn() {
    let max_ip_entries = 100;
    let config = RateLimitConfig {
        max_requests_per_second: 1000,
        burst_size: 100,
        enabled: true,
        max_ip_entries,
        ip_entry_ttl_seconds: 3600, // long TTL: eviction is driven by capacity, not expiry
    };

    let result = time::timeout(Duration::from_secs(30), async move {
        let limiter = IpRateLimiter::new(config);

        // Insert many more distinct IPs than capacity, checking the invariant
        // throughout. The /16 spread guarantees ~5000 unique addresses.
        for i in 0..5000u32 {
            let ip: IpAddr = format!("10.0.{}.{}", (i / 256) % 256, i % 256)
                .parse()
                .unwrap();
            let _ = limiter.check_rate_limit(ip);

            assert!(
                limiter.entry_count() <= max_ip_entries,
                "entry count {} exceeded max {} during churn",
                limiter.entry_count(),
                max_ip_entries
            );
        }

        limiter.cleanup_old_entries();
        assert!(limiter.entry_count() <= max_ip_entries);
    })
    .await;

    assert!(result.is_ok(), "eviction churn deadlocked or timed out");
}

#[tokio::test]
async fn test_rate_limit_headers() {
    let config = RateLimitConfig {
        max_requests_per_second: 5,
        burst_size: 2,
        enabled: true,
        ..Default::default()
    };
    let limiter = arkavo_protocol::RateLimiter::new(config);

    // Get status before any requests
    let status = limiter.get_limit_status();
    assert_eq!(status.limit, 5);
    assert_eq!(status.remaining, 2); // burst size
    assert!(status.reset_at > chrono::Utc::now());

    // Make some requests to consume tokens
    assert!(limiter.check_rate_limit().is_ok());
    assert!(limiter.check_rate_limit().is_ok());

    // Now we should be rate limited
    let result = limiter.check_rate_limit();
    assert!(result.is_err());

    // Note: Since governor doesn't expose exact state, we're using approximations
    // The remaining count will still show burst_size
    let status_after = limiter.get_limit_status();
    assert_eq!(status_after.remaining, 2); // Still shows burst size (approximation)
    assert!(status_after.reset_at > chrono::Utc::now());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_per_ip_rate_limiting_stress() {
    let config = RateLimitConfig {
        max_requests_per_second: 5,
        burst_size: 2,
        enabled: true,
        max_ip_entries: 1000,
        ip_entry_ttl_seconds: 60,
    };

    let limiter = Arc::new(IpRateLimiter::new(config));

    // Simulate multiple IPs making requests concurrently
    let mut handles = vec![];

    for i in 0..10 {
        let limiter_clone = limiter.clone();
        let handle = tokio::spawn(async move {
            let ip: IpAddr = format!("192.168.1.{i}").parse().unwrap();

            let mut allowed = 0;
            let mut blocked = 0;

            // Make 10 rapid requests
            for _ in 0..10 {
                match limiter_clone.check_rate_limit(ip) {
                    Ok(_) => allowed += 1,
                    Err(_) => blocked += 1,
                }
                time::sleep(Duration::from_millis(50)).await;
            }

            (allowed, blocked)
        });
        handles.push(handle);
    }

    // Wait for all tasks
    let mut results = vec![];
    for handle in handles {
        results.push(handle.await.unwrap());
    }

    // Each IP should have some allowed and some blocked
    for (allowed, blocked) in results {
        assert!(allowed > 0, "Should have some allowed requests");
        assert!(blocked > 0, "Should have some blocked requests");
        assert_eq!(allowed + blocked, 10, "Total should be 10");
    }
}
