#![no_main]

use arkavo_protocol::{IpRateLimiter, RateLimitConfig};
use libfuzzer_sys::fuzz_target;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::thread;

// Simplified concurrent fuzz input
#[derive(Debug, Clone)]
struct ConcurrentInput {
    config: RateLimitConfig,
    thread_count: u8,
    requests_per_thread: u16,
}

impl<'a> arbitrary::Arbitrary<'a> for ConcurrentInput {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(ConcurrentInput {
            config: RateLimitConfig {
                max_requests_per_second: u.int_in_range(1..=100)?,
                burst_size: u.int_in_range(1..=20)?,
                enabled: true,
                max_ip_entries: u.int_in_range(50..=500)?,
                ip_entry_ttl_seconds: u.int_in_range(1..=60)?,
            },
            thread_count: u.int_in_range(1..=10)?,
            requests_per_thread: u.int_in_range(10..=100)?,
        })
    }
}

fuzz_target!(|input: ConcurrentInput| {
    let limiter = Arc::new(IpRateLimiter::new(input.config.clone()));
    
    // Spawn threads
    let mut handles = vec![];
    
    for thread_id in 0..input.thread_count {
        let limiter_clone = limiter.clone();
        let requests = input.requests_per_thread;
        
        let handle = thread::spawn(move || {
            let mut allowed = 0;
            let mut blocked = 0;
            
            for i in 0..requests {
                // Each thread uses a mix of its own IPs and shared IPs
                let ip = if i % 3 == 0 {
                    // Shared IP across threads
                    IpAddr::V4(Ipv4Addr::new(10, 0, 0, i as u8 % 10))
                } else {
                    // Thread-specific IP
                    IpAddr::V4(Ipv4Addr::new(192, 168, thread_id, i as u8))
                };
                
                match limiter_clone.check_rate_limit(ip) {
                    Ok(_) => allowed += 1,
                    Err(_) => blocked += 1,
                }
            }
            
            (allowed, blocked)
        });
        
        handles.push(handle);
    }
    
    // Wait for all threads
    let mut total_allowed = 0;
    let mut total_blocked = 0;
    
    for handle in handles {
        let (allowed, blocked) = handle.join().unwrap();
        total_allowed += allowed;
        total_blocked += blocked;
    }
    
    // Verify invariants
    let total_requests = (input.thread_count as usize) * (input.requests_per_thread as usize);
    assert_eq!(total_requests, total_allowed + total_blocked);
    
    // Cleanup and verify count
    limiter.cleanup_old_entries();
    let final_count = limiter.entry_count();
    assert!(
        final_count <= input.config.max_ip_entries,
        "Entry count {} exceeds max {}",
        final_count,
        input.config.max_ip_entries
    );
});