#![no_main]

use arkavo_protocol::{IpRateLimiter, RateLimitConfig};
use libfuzzer_sys::fuzz_target;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

// Fuzz input structure
#[derive(Debug, Clone)]
struct FuzzInput {
    ips: Vec<IpAddr>,
    config: FuzzConfig,
}

#[derive(Debug, Clone)]
struct FuzzConfig {
    max_requests_per_second: u32,
    burst_size: u32,
    max_ip_entries: usize,
    ip_entry_ttl_seconds: u64,
}

impl<'a> arbitrary::Arbitrary<'a> for FuzzInput {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate config
        let config = FuzzConfig {
            max_requests_per_second: u.int_in_range(1..=1000)?,
            burst_size: u.int_in_range(1..=100)?,
            max_ip_entries: u.int_in_range(10..=10000)?,
            ip_entry_ttl_seconds: u.int_in_range(0..=3600)?,
        };

        // Generate IPs - mix of IPv4 and IPv6
        let ip_count = u.int_in_range(1..=1000)?;
        let mut ips = Vec::with_capacity(ip_count);
        
        for _ in 0..ip_count {
            let ip = if u.ratio(1, 2)? {
                // IPv4
                let octets: [u8; 4] = [
                    u.int_in_range(0..=255)? as u8,
                    u.int_in_range(0..=255)? as u8,
                    u.int_in_range(0..=255)? as u8,
                    u.int_in_range(0..=255)? as u8,
                ];
                IpAddr::V4(Ipv4Addr::from(octets))
            } else {
                // IPv6
                let segments: [u16; 8] = [
                    u.int_in_range(0..=65535)? as u16,
                    u.int_in_range(0..=65535)? as u16,
                    u.int_in_range(0..=65535)? as u16,
                    u.int_in_range(0..=65535)? as u16,
                    u.int_in_range(0..=65535)? as u16,
                    u.int_in_range(0..=65535)? as u16,
                    u.int_in_range(0..=65535)? as u16,
                    u.int_in_range(0..=65535)? as u16,
                ];
                IpAddr::V6(Ipv6Addr::from(segments))
            };
            ips.push(ip);
        }

        Ok(FuzzInput { ips, config })
    }
}

fuzz_target!(|input: FuzzInput| {
    // Create rate limiter with fuzzed config
    let config = RateLimitConfig {
        max_requests_per_second: input.config.max_requests_per_second,
        burst_size: input.config.burst_size,
        enabled: true,
        max_ip_entries: input.config.max_ip_entries,
        ip_entry_ttl_seconds: input.config.ip_entry_ttl_seconds,
    };
    
    let limiter = IpRateLimiter::new(config);
    
    // Track statistics
    let mut total_requests = 0;
    let mut allowed = 0;
    let mut blocked = 0;
    
    // Process all IPs
    for ip in &input.ips {
        total_requests += 1;
        
        match limiter.check_rate_limit(*ip) {
            Ok(_) => allowed += 1,
            Err(_) => blocked += 1,
        }
        
        // Occasionally check entry count
        if total_requests % 100 == 0 {
            let count = limiter.entry_count();
            
            // Verify count is within bounds
            assert!(
                count <= input.config.max_ip_entries,
                "Entry count {} exceeds max {}",
                count,
                input.config.max_ip_entries
            );
        }
    }
    
    // Cleanup and verify
    limiter.cleanup_old_entries();
    let final_count = limiter.entry_count();
    
    // Basic sanity checks
    assert_eq!(total_requests, allowed + blocked);
    assert!(final_count <= input.config.max_ip_entries);
    
    // If we had any requests, we should have some entries (unless TTL is 0)
    if total_requests > 0 && input.config.ip_entry_ttl_seconds > 0 {
        // Note: entries might be evicted, so we can't assert > 0
    }
});