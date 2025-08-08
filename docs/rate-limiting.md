# Rate Limiting Configuration

Arkavo Edge implements per-IP rate limiting to protect against abuse and ensure fair resource allocation.

## Configuration

Rate limiting is configured through the `ServerConfig` in the A2A protocol:

```rust
ServerConfig {
    rate_limit: RateLimitConfig {
        max_requests_per_second: 100,
        burst_size: 10,
        enabled: true,
        max_ip_entries: 10_000,
        ip_entry_ttl_seconds: 3600,
    },
    trusted_proxies: vec!["10.0.0.1".to_string()],
    enable_x_forwarded_for: true,
    // ... other config
}
```

## Configuration Parameters

### Rate Limiting
- `max_requests_per_second`: Maximum sustained request rate per IP
- `burst_size`: Allows temporary spikes above the limit
- `enabled`: Toggle rate limiting on/off
- `max_ip_entries`: Maximum number of IP addresses to track (LRU eviction)
- `ip_entry_ttl_seconds`: How long to keep IP entries before eviction

### Proxy Support
- `trusted_proxies`: List of proxy IPs to trust for X-Forwarded-For headers
- `enable_x_forwarded_for`: Enable parsing of proxy headers

## Headers Supported

When behind a proxy, the following headers are parsed (in order of preference):
1. `X-Forwarded-For`: Standard proxy header (leftmost IP is used)
2. `X-Real-IP`: Alternative header used by some proxies
3. `Forwarded`: RFC 7239 standard header

## Environment Variables

```bash
# Basic rate limiting
export A2A_RATE_LIMIT_MAX_RPS=100
export A2A_RATE_LIMIT_BURST_SIZE=10
export A2A_RATE_LIMIT_ENABLED=true

# IP tracking
export A2A_RATE_LIMIT_MAX_IP_ENTRIES=10000
export A2A_RATE_LIMIT_IP_TTL_SECONDS=3600

# Proxy configuration
export A2A_TRUSTED_PROXIES="10.0.0.1,10.0.0.2"
export A2A_ENABLE_X_FORWARDED_FOR=true
```

## Metrics

Rate limiting metrics are tracked via the `MetricsCollector`:
- `rate_limit_blocked`: Counter of blocked requests per IP
- `rate_limit_entries`: Gauge of currently tracked IPs

## Example Configurations

### Development Environment
```toml
[server.rate_limit]
max_requests_per_second = 1000
burst_size = 100
enabled = true
max_ip_entries = 100
ip_entry_ttl_seconds = 60
```

### Production with Load Balancer
```toml
[server]
trusted_proxies = ["10.0.0.0/8"]
enable_x_forwarded_for = true

[server.rate_limit]
max_requests_per_second = 100
burst_size = 10
enabled = true
max_ip_entries = 50000
ip_entry_ttl_seconds = 3600
```

### High-Traffic API
```toml
[server.rate_limit]
max_requests_per_second = 500
burst_size = 50
enabled = true
max_ip_entries = 100000
ip_entry_ttl_seconds = 1800
```

## Implementation Details

The rate limiter uses:
- **Token bucket algorithm** for rate limiting
- **LRU eviction** when IP limit is reached
- **Concurrent hash map** for lock-free IP tracking
- **Background cleanup task** runs every 60 seconds

## Testing

Run the integration tests:
```bash
cargo test -p arkavo-protocol per_ip_rate_limit_test
```

## Troubleshooting

### All requests are rate limited
- Check if `max_requests_per_second` is too low
- Verify proxy headers are being parsed correctly
- Ensure `trusted_proxies` includes your proxy IPs

### Memory usage is high
- Reduce `max_ip_entries`
- Lower `ip_entry_ttl_seconds`
- Monitor the number of tracked IPs via metrics

### X-Forwarded-For not working
- Verify `enable_x_forwarded_for` is true
- Ensure proxy IP is in `trusted_proxies` list
- Check proxy is sending the header correctly