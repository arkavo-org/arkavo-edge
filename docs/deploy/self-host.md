# Self-Hosting Arkavo Edge

This guide covers deploying and operating Arkavo Edge with the Bidirectional Chat Protocol v2 in production environments.

## Prerequisites

- Rust 1.75+ (for building from source)
- SQLite 3.35+ (for session persistence)
- TLS certificates (for secure communication)
- JWT signing keys (for authentication)

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/arkavo-org/arkavo-edge.git
cd arkavo-edge

# Build release binary
cargo build --release --features chat-v2

# Binary will be at target/release/arkavo
```

### Using Docker

```dockerfile
FROM rust:1.75 as builder
WORKDIR /app
COPY . .
RUN cargo build --release --features chat-v2

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    ca-certificates \
    sqlite3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/arkavo /usr/local/bin/arkavo
EXPOSE 8080
CMD ["arkavo", "serve"]
```

## Configuration

### Basic Configuration

Create a configuration file at `/etc/arkavo/config.yaml`:

```yaml
server:
  host: 0.0.0.0
  port: 8080
  
auth:
  method: jwt
  jwt_secret: ${JWT_SECRET}
  jwt_audience: arkavo-chat
  jwt_issuer: auth-service
  
persistence:
  enabled: true
  db_path: /var/lib/arkavo/sessions.db
  retention_hours: 24
  
tls:
  enabled: true
  cert_path: /etc/arkavo/certs/server.crt
  key_path: /etc/arkavo/certs/server.key
  ca_path: /etc/arkavo/certs/ca.crt
  
chat:
  max_inflight_deltas: 100
  session_ttl_seconds: 3600
  max_context_length: 4096
  
rate_limiting:
  enabled: true
  requests_per_second: 100
  burst_size: 200
```

### Environment Variables

All configuration can be overridden via environment variables:

```bash
# Authentication
export ARKAVO_JWT_SECRET="your-secret-key"
export ARKAVO_JWT_AUDIENCE="arkavo-chat"
export ARKAVO_JWT_ISSUER="auth-service"

# Persistence
export ARKAVO_SESSION_DB_PATH="/var/lib/arkavo/sessions.db"
export ARKAVO_SESSION_RETENTION_HOURS="24"

# Performance
export ARKAVO_MAX_INFLIGHT_DELTAS="100"
export ARKAVO_SESSION_TTL_SECONDS="3600"

# TLS
export ARKAVO_TLS_CERT="/etc/arkavo/certs/server.crt"
export ARKAVO_TLS_KEY="/etc/arkavo/certs/server.key"
export ARKAVO_TLS_CA="/etc/arkavo/certs/ca.crt"
```

## Deployment Architectures

### Single Instance

Suitable for development and small deployments:

```bash
arkavo serve \
  --config /etc/arkavo/config.yaml \
  --log-level info
```

### High Availability

For production deployments with multiple instances:

```yaml
# docker-compose.yml
version: '3.8'

services:
  arkavo-1:
    image: arkavo:latest
    environment:
      - ARKAVO_INSTANCE_ID=1
      - ARKAVO_JWT_SECRET=${JWT_SECRET}
    volumes:
      - sessions-db:/var/lib/arkavo
      - ./certs:/etc/arkavo/certs:ro
    networks:
      - arkavo-net
    
  arkavo-2:
    image: arkavo:latest
    environment:
      - ARKAVO_INSTANCE_ID=2
      - ARKAVO_JWT_SECRET=${JWT_SECRET}
    volumes:
      - sessions-db:/var/lib/arkavo
      - ./certs:/etc/arkavo/certs:ro
    networks:
      - arkavo-net
  
  nginx:
    image: nginx:alpine
    ports:
      - "443:443"
    volumes:
      - ./nginx.conf:/etc/nginx/nginx.conf:ro
      - ./certs:/etc/nginx/certs:ro
    depends_on:
      - arkavo-1
      - arkavo-2
    networks:
      - arkavo-net

volumes:
  sessions-db:

networks:
  arkavo-net:
```

### Kubernetes Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: arkavo-edge
spec:
  replicas: 3
  selector:
    matchLabels:
      app: arkavo-edge
  template:
    metadata:
      labels:
        app: arkavo-edge
    spec:
      containers:
      - name: arkavo
        image: arkavo:latest
        ports:
        - containerPort: 8080
        env:
        - name: ARKAVO_JWT_SECRET
          valueFrom:
            secretKeyRef:
              name: arkavo-secrets
              key: jwt-secret
        volumeMounts:
        - name: sessions
          mountPath: /var/lib/arkavo
        - name: certs
          mountPath: /etc/arkavo/certs
          readOnly: true
        resources:
          requests:
            memory: "256Mi"
            cpu: "250m"
          limits:
            memory: "512Mi"
            cpu: "500m"
      volumes:
      - name: sessions
        persistentVolumeClaim:
          claimName: arkavo-sessions
      - name: certs
        secret:
          secretName: arkavo-tls
---
apiVersion: v1
kind: Service
metadata:
  name: arkavo-edge
spec:
  selector:
    app: arkavo-edge
  ports:
  - port: 8080
    targetPort: 8080
  type: ClusterIP
```

## Security Setup

### JWT Configuration

1. **Generate signing keys**:
```bash
# For HS256 (symmetric)
openssl rand -base64 32 > jwt-secret.key

# For RS256 (asymmetric)
openssl genrsa -out jwt-private.pem 2048
openssl rsa -in jwt-private.pem -pubout -out jwt-public.pem
```

2. **Configure auth backend**:
```yaml
auth:
  method: jwt
  algorithm: RS256
  public_key_path: /etc/arkavo/keys/jwt-public.pem
  audience: arkavo-chat
  issuer: your-auth-service
```

### TLS/mTLS Setup

1. **Generate certificates**:
```bash
# Generate CA
openssl genrsa -out ca.key 4096
openssl req -new -x509 -days 365 -key ca.key -out ca.crt

# Generate server certificate
openssl genrsa -out server.key 2048
openssl req -new -key server.key -out server.csr
openssl x509 -req -days 365 -in server.csr -CA ca.crt -CAkey ca.key -out server.crt

# For mTLS, generate client certificates
openssl genrsa -out client.key 2048
openssl req -new -key client.key -out client.csr
openssl x509 -req -days 365 -in client.csr -CA ca.crt -CAkey ca.key -out client.crt
```

2. **Configure TLS**:
```yaml
tls:
  enabled: true
  cert_path: /etc/arkavo/certs/server.crt
  key_path: /etc/arkavo/certs/server.key
  ca_path: /etc/arkavo/certs/ca.crt
  verify_client: true  # Enable for mTLS
  minimum_version: TLS1.3
```

## Database Management

### Initial Setup

```bash
# Create database directory
mkdir -p /var/lib/arkavo

# Initialize database (automatic on first run)
arkavo db init --path /var/lib/arkavo/sessions.db
```

### Backup and Restore

```bash
# Backup
sqlite3 /var/lib/arkavo/sessions.db ".backup /backup/sessions-$(date +%Y%m%d).db"

# Restore
sqlite3 /var/lib/arkavo/sessions.db ".restore /backup/sessions-20240115.db"
```

### Maintenance

```bash
# Vacuum database (reclaim space)
sqlite3 /var/lib/arkavo/sessions.db "VACUUM;"

# Clean old sessions
arkavo db cleanup --older-than 7d --path /var/lib/arkavo/sessions.db

# Analyze for query optimization
sqlite3 /var/lib/arkavo/sessions.db "ANALYZE;"
```

## Monitoring

### Health Checks

```bash
# HTTP health endpoint
curl https://your-server/.well-known/agent.json

# Response
{
  "status": "healthy",
  "version": "2.0.0",
  "uptime": 3600,
  "sessions_active": 42
}
```

### Metrics

Prometheus metrics available at `/metrics`:

```prometheus
# Session metrics
arkavo_sessions_active 42
arkavo_sessions_total 1234
arkavo_sessions_duration_seconds_bucket{le="60"} 100
arkavo_sessions_duration_seconds_bucket{le="300"} 200

# Message metrics
arkavo_messages_sent_total 5678
arkavo_messages_received_total 4321
arkavo_deltas_sent_total 98765

# Back-pressure metrics
arkavo_backpressure_pauses_total 10
arkavo_inflight_deltas 25

# Performance metrics
arkavo_request_duration_seconds{method="chat_open"} 0.025
arkavo_llm_generation_duration_seconds 1.234
```

### Logging

Configure logging levels:

```yaml
logging:
  level: info  # debug, info, warn, error
  format: json  # json, pretty
  output: /var/log/arkavo/arkavo.log
  
  # Per-module configuration
  modules:
    arkavo_protocol: debug
    arkavo_llm: info
    arkavo_auth: debug
```

Example log output:
```json
{
  "timestamp": "2024-01-15T10:30:00Z",
  "level": "INFO",
  "module": "arkavo_protocol::chat_session",
  "message": "Session created",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "user": "user123",
  "scopes": ["chat.read", "chat.write"]
}
```

## Performance Tuning

### Connection Pooling

```yaml
database:
  max_connections: 20
  min_connections: 5
  connection_timeout: 30
  idle_timeout: 600
```

### Buffer Configuration

```yaml
buffers:
  message_channel_size: 64
  delta_channel_size: 512
  max_message_size: 1048576  # 1MB
```

### Worker Threads

```yaml
runtime:
  worker_threads: 4  # Default: CPU cores
  blocking_threads: 16
  stack_size: 2097152  # 2MB
```

## Troubleshooting

### Common Issues

1. **High memory usage**
   - Check for session leaks: `arkavo debug sessions --active`
   - Reduce session TTL
   - Enable aggressive cleanup

2. **Slow response times**
   - Check database performance: `arkavo db analyze`
   - Review back-pressure settings
   - Enable connection pooling

3. **Authentication failures**
   - Verify JWT secret/keys match
   - Check token expiration
   - Review audience/issuer configuration

4. **WebSocket disconnections**
   - Increase keep-alive interval
   - Check proxy timeout settings
   - Review TLS configuration

### Debug Commands

```bash
# Show active sessions
arkavo debug sessions --active

# Test authentication
arkavo debug auth --token "eyJhbGc..."

# Database statistics
arkavo db stats --path /var/lib/arkavo/sessions.db

# Performance profiling
arkavo debug profile --duration 60s
```

## Backup Strategy

### Automated Backups

```bash
#!/bin/bash
# /etc/arkavo/backup.sh

BACKUP_DIR="/backup/arkavo"
DB_PATH="/var/lib/arkavo/sessions.db"
RETENTION_DAYS=7

# Create backup
sqlite3 $DB_PATH ".backup $BACKUP_DIR/sessions-$(date +%Y%m%d-%H%M%S).db"

# Remove old backups
find $BACKUP_DIR -name "sessions-*.db" -mtime +$RETENTION_DAYS -delete

# Upload to S3 (optional)
aws s3 sync $BACKUP_DIR s3://your-bucket/arkavo-backups/
```

Add to crontab:
```cron
0 */6 * * * /etc/arkavo/backup.sh
```

## Scaling Guidelines

### Vertical Scaling
- **Memory**: 256MB minimum, 512MB recommended per instance
- **CPU**: 0.5 cores minimum, 1 core recommended
- **Disk**: 10GB for database (depends on retention)

### Horizontal Scaling
- Use shared storage for session database
- Configure session affinity in load balancer
- Consider using PostgreSQL for multi-instance deployments

### Load Balancer Configuration

nginx example:
```nginx
upstream arkavo_backend {
    least_conn;
    server arkavo-1:8080 max_fails=3 fail_timeout=30s;
    server arkavo-2:8080 max_fails=3 fail_timeout=30s;
    server arkavo-3:8080 max_fails=3 fail_timeout=30s;
}

server {
    listen 443 ssl http2;
    
    ssl_certificate /etc/nginx/certs/server.crt;
    ssl_certificate_key /etc/nginx/certs/server.key;
    
    location / {
        proxy_pass http://arkavo_backend;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        
        # WebSocket settings
        proxy_read_timeout 3600;
        proxy_send_timeout 3600;
    }
}
```

## Migration Guide

### From v1 to v2

1. **Update configuration**:
   - Add JWT configuration
   - Configure persistence
   - Update TLS settings

2. **Database migration**:
```bash
# Export v1 data
arkavo-v1 export --format json > sessions.json

# Import to v2
arkavo import --format json --input sessions.json
```

3. **Client updates**:
   - Update to handle new delta format
   - Implement metrics acknowledgment
   - Add JWT token to requests

## Support

- GitHub Issues: https://github.com/arkavo-org/arkavo-edge/issues
- Documentation: https://docs.arkavo.org
- Community Discord: https://discord.gg/arkavo