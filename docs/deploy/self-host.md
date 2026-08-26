# Self-Hosting Arkavo Edge

This guide covers deploying and operating Arkavo Edge's AG-UI web gateway in
production environments.

Arkavo Edge is configured entirely through environment variables — there is no
configuration file. The long-running server mode is the AG-UI web gateway,
started with `arkavo ui`. There are no `arkavo serve` or `arkavo db`
subcommands, and the gateway does not expose a Prometheus endpoint; see the
monitoring section below for what is actually available.

## Prerequisites

- Rust (stable) for building from source, or Docker for the container image
- An API key for at least one remote LLM provider (Gemini, OpenAI-compatible,
  DeepSeek, xAI) — container/self-host builds ship without local inference
- A reverse proxy (nginx, ingress, etc.) if you need TLS or authentication —
  the gateway itself is unauthenticated (see Security)

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/arkavo-org/arkavo-edge.git
cd arkavo-edge

# Build the binary with the web gateway and remote LLM providers
cargo build --release -p arkavo \
  --no-default-features \
  --features memory,mdns,mcp-tools,llm-remote,web-ui

# Binary will be at target/release/arkavo
```

### Using Docker

The repository ships a root `Dockerfile` (documented in
[container.md](container.md)) that builds exactly this feature set. A minimal
equivalent:

```dockerfile
FROM rust:1-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release -p arkavo \
    --no-default-features \
    --features memory,mdns,mcp-tools,llm-remote,web-ui

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

ENV ARKAVO_SKIP_FIRST_RUN=1
COPY --from=builder /app/target/release/arkavo /usr/local/bin/arkavo
EXPOSE 7700
ENTRYPOINT ["arkavo"]
CMD ["ui"]
```

`ARKAVO_SKIP_FIRST_RUN=1` is required in containers: it skips the interactive
first-run flow that downloads a local model, which is meaningless in a
no-inference image.

## Configuration

All configuration is via environment variables. There is no config file.

```bash
# LLM provider credentials (at least one required)
export GEMINI_API_KEY="..."       # Gemini
export OPENAI_API_KEY="..."       # OpenAI-compatible providers
export DEEPSEEK_API_KEY="..."     # DeepSeek

# Container / unattended operation
export ARKAVO_SKIP_FIRST_RUN=1    # Skip interactive first-run model download

# Logging
export ARKAVO_DEBUG=1             # General debug logging
export ARKAVO_DEBUG_CHAT=1        # Chat/template/token debug logging
```

Never bake API keys into the image; pass them with `docker run -e ...` or your
orchestrator's secret mechanism.

## Running the Server

```bash
# Start the AG-UI web gateway on the default port (7700)
arkavo ui

# Custom port
arkavo ui --port 8080
```

The gateway serves:

- `/` — the web UI (plus `/static/*` assets)
- `/ws` — AG-UI WebSocket event stream
- `/api/agent` and `/api/agent/capabilities` — agent execution API
- `/agent/:id` and `/api/dataflow/*path` — proxy routes
- `/debug` — debug WebSocket feed

The API routes are rate-limited per source IP; static assets are not.

## Deployment Architectures

### Single Instance

Suitable for development and small deployments:

```bash
GEMINI_API_KEY=... ARKAVO_SKIP_FIRST_RUN=1 arkavo ui --port 7700
```

### Docker Compose

```yaml
# docker-compose.yml
services:
  arkavo:
    image: arkavo-edge:latest
    command: ["ui", "--port", "7700"]
    environment:
      - GEMINI_API_KEY=${GEMINI_API_KEY}
      - ARKAVO_SKIP_FIRST_RUN=1
    volumes:
      - arkavo-data:/data
    working_dir: /data
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
      - arkavo
    networks:
      - arkavo-net

volumes:
  arkavo-data:

networks:
  arkavo-net:
```

The `working_dir` matters: persistent state (SQLite memory/event stores) lives
under `.arkavo/` relative to the process working directory, so point it at a
mounted volume to survive container replacement.

### Kubernetes Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: arkavo-edge
spec:
  replicas: 1
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
        image: arkavo-edge:latest
        args: ["ui", "--port", "7700"]
        ports:
        - containerPort: 7700
        env:
        - name: GEMINI_API_KEY
          valueFrom:
            secretKeyRef:
              name: arkavo-secrets
              key: gemini-api-key
        - name: ARKAVO_SKIP_FIRST_RUN
          value: "1"
        volumeMounts:
        - name: data
          mountPath: /data
        workingDir: /data
        resources:
          requests:
            memory: "256Mi"
            cpu: "250m"
          limits:
            memory: "512Mi"
            cpu: "500m"
      volumes:
      - name: data
        persistentVolumeClaim:
          claimName: arkavo-data
---
apiVersion: v1
kind: Service
metadata:
  name: arkavo-edge
spec:
  selector:
    app: arkavo-edge
  ports:
  - port: 80
    targetPort: 7700
  type: ClusterIP
```

The memory store is workspace-local, so run `replicas: 1` with a PVC, or
accept that each replica has its own independent state. Terminate TLS at the
ingress.

## Security

The AG-UI gateway binds `0.0.0.0` with **no authentication**
(`crates/arkavo-agui/src/gateway.rs:489`). Anyone who can reach the port can
drive the agent. Until gateway authentication lands:

- Never publish the port directly to untrusted networks.
- Put the gateway behind a reverse proxy that enforces TLS and
  authentication (OAuth proxy, basic auth, mTLS — your choice).
- Or restrict exposure to trusted networks (loopback, VPN, cluster-internal).

The gateway does apply security headers and per-IP rate limiting
(`arkavo_protocol::ip_rate_limit_middleware`), but those are not a substitute
for authentication.

### Reverse Proxy Example (nginx)

```nginx
upstream arkavo_backend {
    server arkavo:7700 max_fails=3 fail_timeout=30s;
}

server {
    listen 443 ssl http2;

    ssl_certificate /etc/nginx/certs/server.crt;
    ssl_certificate_key /etc/nginx/certs/server.key;

    # Enforce authentication here, e.g.:
    # auth_basic "arkavo";
    # auth_basic_user_file /etc/nginx/.htpasswd;

    location / {
        proxy_pass http://arkavo_backend;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # WebSocket connections are long-lived
        proxy_read_timeout 3600;
        proxy_send_timeout 3600;
    }
}
```

## Persistence

State lives in SQLite databases under `.arkavo/memory_server/` relative to the
working directory (memories, event store, TDF audit, federated memory). The
databases are created automatically on first use — there is no init command.

### Backup and Restore

```bash
# Backup (from the working directory of the running instance)
sqlite3 .arkavo/memory_server/memories.db ".backup /backup/memories-$(date +%Y%m%d).db"

# Restore
sqlite3 .arkavo/memory_server/memories.db ".restore /backup/memories-20260101.db"
```

For unattended backups, stop writes (or rely on SQLite's online backup) and
copy the whole `.arkavo/` directory; upload to object storage as needed.

## Monitoring

There is no Prometheus `/metrics` endpoint and no HTTP health endpoint. What
exists today:

- **Logs**: the gateway logs to stdout; increase verbosity with
  `ARKAVO_DEBUG=1` and `ARKAVO_DEBUG_CHAT=1`. Collect stdout with your
  container/platform log pipeline.
- **Debug WebSocket**: `/debug` streams internal events for live inspection
  from the web UI.
- **Health reporters**: internal component health (router connectivity,
  learning pipeline, UI generator) is surfaced as AG-UI events over the
  WebSocket, not as an HTTP endpoint.

For container orchestration health checks, use a TCP check against the
gateway port:

```yaml
readinessProbe:
  tcpSocket:
    port: 7700
  initialDelaySeconds: 5
  periodSeconds: 10
```

## Troubleshooting

- **Gateway unreachable externally**: it binds `0.0.0.0`, so check proxy,
  firewall, and port-mapping configuration first.
- **Agent requests fail**: verify the provider API key env var is set and
  valid; run with `ARKAVO_DEBUG=1` for provider error detail.
- **State lost after container restart**: the working directory was not a
  mounted volume — set `working_dir`/`workingDir` to a persistent mount.
- **Interactive first-run prompt in a container**: set
  `ARKAVO_SKIP_FIRST_RUN=1`.

## Scaling Guidelines

- **Memory**: 256MB minimum, 512MB recommended per instance
- **CPU**: 0.5 cores minimum, 1 core recommended
- **Disk**: sized for the `.arkavo/` SQLite stores and model caches
- **Horizontal scaling**: not currently meaningful for shared state — memory
  is workspace-local SQLite. Run one replica per workspace, or front
  independent instances with your own routing.

## Support

- GitHub Issues: https://github.com/arkavo-org/arkavo-edge/issues
