# Claude Code Agent Configuration

name: claude-code-agent
type: coding
port: 8345

## Model Configuration

model: claude-3-sonnet-20240229
# For DeepSeek, use: deepseek-chat

## Capabilities

capabilities:
  - code_generation
  - code_analysis
  - code_improvement
  - test_generation
  - documentation
  - claude_code

## Claude Code Configuration

claude_code:
  enabled: true
  workspace_root: ./workspace
  budget_tokens: 200000
  
  # Tool permissions
  tools:
    read: true
    write: true
    exec: false      # Disabled for safety by default
    web_search: true
  
  # File access patterns
  allow_globs:
    - "**/*.js"
    - "**/*.ts"
    - "**/*.tsx"
    - "**/*.jsx"
    - "**/*.py"
    - "**/*.rs"
    - "**/*.go"
    - "**/*.java"
    - "**/*.c"
    - "**/*.cpp"
    - "**/*.h"
    - "**/*.hpp"
    - "**/*.md"
    - "**/*.json"
    - "**/*.yaml"
    - "**/*.yml"
    - "**/*.toml"
    - "**/*.xml"
    - "**/*.html"
    - "**/*.css"
    - "**/*.scss"
    - "**/Dockerfile"
    - "**/Makefile"
    - "**/package.json"
    - "**/Cargo.toml"
  
  deny_globs:
    - "**/.env"
    - "**/.env.*"
    - "**/secrets/**"
    - "**/*.key"
    - "**/*.pem"
    - "**/*.p12"
    - "**/node_modules/**"
    - "**/target/**"
    - "**/.git/**"
  
  # Rate limiting
  rate_limit:
    max_attempts: 3
    backoff_ms: 800
  
  # Session management
  session_ttl: 3600
  
  # Logging
  log_redaction: true

## A2A Protocol Configuration

a2a:
  enabled: true
  discovery:
    mdns: true
    static_peers:
      - "project-manager:8342"
      - "testing-agent:8344"
  
  security:
    rate_limit: 100  # requests per minute
    require_auth: false
  
  capabilities_broadcast:
    interval: 30  # seconds
    include:
      - code_generation
      - claude_code

## MCP Servers

mcp_servers: []
# Can add MCP servers if needed:
# - name: git-mcp
#   command: npx
#   args: [@modelcontextprotocol/server-git]

## WebSocket Configuration

websocket:
  enabled: true
  ping_interval: 30
  max_connections: 10

## Logging

logging:
  level: info
  file: logs/claude-code-agent.log
  format: json

## Health Check

health_check:
  enabled: true
  interval: 60
  endpoint: /.well-known/agent.json

## Budget Tracking

budget:
  enabled: true
  limits:
    hourly: 1.00    # $1.00 per hour
    daily: 10.00    # $10.00 per day
    monthly: 100.00 # $100.00 per month
  
  alerts:
    warning_percent: 80
    critical_percent: 95

## Telemetry

telemetry:
  enabled: false
  endpoint: http://localhost:4317
  service_name: claude-code-agent

## Environment Variable Overrides

# These can be set via environment variables:
# ANTHROPIC_API_KEY - Claude API key
# ANTHROPIC_BASE_URL - Alternative API endpoint (e.g., DeepSeek)
# ANTHROPIC_AUTH_TOKEN - Alternative auth token
# ANTHROPIC_MODEL - Override model selection
# ANTHROPIC_SMALL_FAST_MODEL - Fast model for simple tasks