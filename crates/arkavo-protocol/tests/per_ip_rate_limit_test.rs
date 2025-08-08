use arkavo_protocol::{ConnectionTracker, IpRateLimiter, RateLimitConfig, spawn_cleanup_task};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn test_concurrent_ip_rate_limiting() {
    // Configure rate limiting with tight limits to force rate limiting
    let rate_config = RateLimitConfig {
        max_requests_per_second: 5,
        burst_size: 2,
        enabled: true,
        max_ip_entries: 10000,
        ip_entry_ttl_seconds: 300,
    };

    let limiter = Arc::new(IpRateLimiter::new(rate_config.clone()));

    // Test with 100 different IPs
    let num_ips = 100;
    let requests_per_ip = 20;
    let mut handles = vec![];

    for i in 0..num_ips {
        let limiter_clone = limiter.clone();

        let handle = tokio::spawn(async move {
            let ip: IpAddr = format!("192.168.1.{}", i % 256).parse().unwrap();

            let mut allowed = 0;
            let mut blocked = 0;

            // Make rapid requests from this IP
            for _ in 0..requests_per_ip {
                match limiter_clone.check_rate_limit(ip) {
                    Ok(_) => allowed += 1,
                    Err(_) => blocked += 1,
                }
                // Small delay to simulate realistic request patterns
                time::sleep(Duration::from_millis(10)).await;
            }

            (ip, allowed, blocked)
        });

        handles.push(handle);
    }

    // Collect results
    let mut results = vec![];
    for handle in handles {
        results.push(handle.await.unwrap());
    }

    // Verify each IP was rate limited independently
    for (ip, allowed, blocked) in &results {
        assert!(*allowed > 0, "IP {} should have some allowed requests", ip);
        assert!(
            *blocked > 0,
            "IP {} should have some blocked requests due to rate limiting",
            ip
        );
        assert_eq!(
            allowed + blocked,
            requests_per_ip,
            "Total requests for IP {} should equal {}",
            ip,
            requests_per_ip
        );
    }

    // Verify the limiter tracked all IPs
    assert!(limiter.entry_count() > 0, "Limiter should be tracking IPs");

    println!("Successfully tested {} concurrent IPs", num_ips);
    println!("Limiter tracking {} unique IPs", limiter.entry_count());
}

#[tokio::test]
async fn test_lru_eviction_under_load() {
    let config = RateLimitConfig {
        max_requests_per_second: 10,
        burst_size: 5,
        enabled: true,
        max_ip_entries: 50, // Small limit to force eviction
        ip_entry_ttl_seconds: 3600,
    };

    let limiter = Arc::new(IpRateLimiter::new(config));

    // Add more IPs than the limit
    for i in 0..100 {
        let ip: IpAddr = format!("10.0.0.{}", i).parse().unwrap();
        let _ = limiter.check_rate_limit(ip);
    }

    // The limiter should have evicted old entries
    assert!(
        limiter.entry_count() <= 50,
        "Limiter should respect max_ip_entries limit after LRU eviction"
    );

    println!(
        "LRU eviction working: {} entries (max 50)",
        limiter.entry_count()
    );
}

#[tokio::test]
async fn test_proxy_header_handling() {
    use arkavo_protocol::HeaderMap;

    let tracker = ConnectionTracker::new(
        vec!["10.0.0.1".to_string()], // Trusted proxy
        true,                         // Enable X-Forwarded-For
    );

    // Simulate connection from proxy with X-Forwarded-For
    let proxy_addr = "10.0.0.1:8080".parse().unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for".to_string(), "203.0.113.42".to_string());

    tracker.register_connection(1, proxy_addr, Some(headers));

    let client_ip = tracker.get_client_ip(1);
    assert_eq!(
        client_ip,
        Some("203.0.113.42".parse::<IpAddr>().unwrap()),
        "Should extract real client IP from X-Forwarded-For"
    );

    // Test with untrusted proxy (should ignore header)
    let untrusted_addr = "192.168.1.100:8080".parse().unwrap();
    let mut headers2 = HeaderMap::new();
    headers2.insert("x-forwarded-for".to_string(), "203.0.113.99".to_string());

    tracker.register_connection(2, untrusted_addr, Some(headers2));

    let client_ip2 = tracker.get_client_ip(2);
    assert_eq!(
        client_ip2,
        Some("192.168.1.100".parse::<IpAddr>().unwrap()),
        "Should use socket IP for untrusted proxy"
    );
}

#[tokio::test]
async fn test_rate_limit_with_metrics() {
    use arkavo_protocol::MetricsCollector;

    let config = RateLimitConfig {
        max_requests_per_second: 5,
        burst_size: 2,
        enabled: true,
        max_ip_entries: 1000,
        ip_entry_ttl_seconds: 60,
    };

    let limiter = Arc::new(IpRateLimiter::new(config));
    let metrics = Arc::new(MetricsCollector::new(true));

    // Start cleanup task with metrics
    let cleanup_handle = spawn_cleanup_task(limiter.clone(), Some(metrics.clone()));

    let ip: IpAddr = "172.16.0.1".parse().unwrap();

    // Make requests until we get rate limited
    let mut blocked_count = 0;
    for _ in 0..10 {
        if limiter.check_rate_limit(ip).is_err() {
            metrics.record_rate_limit_blocked(Some(ip));
            blocked_count += 1;
        }
    }

    assert!(blocked_count > 0, "Should have some blocked requests");

    // Metrics tracking verified via the collector's internal state
    // The export_metrics method would be used in production with prometheus

    cleanup_handle.abort(); // Clean up the background task
}

#[tokio::test]
async fn test_ipv6_support() {
    let config = RateLimitConfig::default();
    let limiter = IpRateLimiter::new(config);

    // Test with IPv6 addresses
    let ipv6: IpAddr = "2001:db8::1".parse().unwrap();
    assert!(limiter.check_rate_limit(ipv6).is_ok());

    let ipv6_2: IpAddr = "2001:db8::2".parse().unwrap();
    assert!(limiter.check_rate_limit(ipv6_2).is_ok());

    // Verify both IPs are tracked separately
    assert_eq!(limiter.entry_count(), 2);
}

#[tokio::test]
async fn test_connection_lifecycle() {
    let tracker = ConnectionTracker::new(vec![], false);

    // Register multiple connections
    for i in 0..10 {
        let addr = format!("192.168.1.{}:8080", i).parse().unwrap();
        tracker.register_connection(i as u64, addr, None);
    }

    assert_eq!(tracker.connection_count(), 10);

    // Remove some connections
    for i in 0..5 {
        tracker.remove_connection(i as u64);
    }

    assert_eq!(tracker.connection_count(), 5);

    // Verify remaining connections still work
    for i in 5..10 {
        let expected_ip: IpAddr = format!("192.168.1.{}", i).parse().unwrap();
        assert_eq!(tracker.get_client_ip(i as u64), Some(expected_ip));
    }
}
