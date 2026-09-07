# Self-Hosting Arkavo Edge

This guide covers deploying and operating Arkavo Edge's AG-UI web gateway in
production environments.

Arkavo Edge is configured entirely through environment variables — there is no
configuration file. The long-running server mode is the AG-UI web gateway,
started with `arkavo ui`. There are no `arkavo serve` or `arkavo db`
subcommands, and the gateway does not expose a Prometheus endpoint; see the
monitoring section below for what is actually available.

## Prerequisites

- Rust (stable), CMake, and ccache for building local inference from source
- Provisioned local models and enough device memory to run them
- Optional cloud credentials to augment local models
- A reverse proxy (nginx, ingress, etc.) if you need TLS or authentication —
  the gateway itself is unauthenticated (see Security)

## Installation

### From Source

```bash
# Clone the repository
git clone --recurse-submodules https://github.com/arkavo-org/arkavo-edge.git
cd arkavo-edge

# Build the local agent harness with optional cloud augmentation
cargo build --release -p arkavo \
  --no-default-features \
  --features llama-cpp,memory,mdns,mcp-tools,openai,web-ui

# Binary will be at target/release/arkavo
```

### Using Docker

The root [container image](container.md) is utility-only and does not support
agent inference. Container examples below require an image built with local
inference support and a mounted, provisioned model cache. Cloud credentials
alone cannot start the harness. `ARKAVO_SKIP_FIRST_RUN=1` suppresses interactive
setup; it never waives the local model requirement.

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

The AG-UI gateway defaults to loopback-only binding
(`crates/arkavo-agui/src/gateway_bind.rs`); set `ARKAVO_AGUI_BIND` (e.g. to
`0.0.0.0`) to opt out and listen on another interface. The container image
sets `ARKAVO_AGUI_BIND=0.0.0.0` so the port you publish is actually
reachable — the container's network namespace is the real isolation
boundary, not the gateway's own bind address. Either way the gateway has
**no authentication**: anyone who can reach the listening interface
(loopback, a published container port, or a wider bind on bare metal) can
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

- **Gateway unreachable externally**: it binds loopback only by default —
  set `ARKAVO_AGUI_BIND=0.0.0.0` (the shipped container image sets this
  already) before checking proxy, firewall, and port-mapping configuration.
- **Agent requests fail**: verify that the build supports local inference and
  that local models are provisioned. For cloud augmentation failures, verify
  provider credentials and cloud policy.
- **State lost after container restart**: the working directory was not a
  mounted volume — set `working_dir`/`workingDir` to a persistent mount.
- **Interactive first-run prompt in a container**: set
  `ARKAVO_SKIP_FIRST_RUN=1`.

## Scaling Guidelines

- **Memory**: include the selected local models and their inference working sets
- **CPU**: size for local inference throughput and latency
- **Disk**: sized for the `.arkavo/` SQLite stores and model caches
- **Horizontal scaling**: not currently meaningful for shared state — memory
  is workspace-local SQLite. Run one replica per workspace, or front
  independent instances with your own routing.

## Support

- GitHub Issues: https://github.com/arkavo-org/arkavo-edge/issues
