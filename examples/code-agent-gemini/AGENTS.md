# Gemini Code Agent Configuration

name: gemini-code-agent
type: coding
port: 8346

## Model Configuration

model: gemini-3.5-flash
# For deeper reasoning: gemini-3-pro-preview
# Legacy alias: gemini-flash-latest

## Capabilities

capabilities:
  - code_generation
  - code_analysis
  - code_improvement
  - test_generation
  - documentation
  - code_search
  - security_scanning

## MCP Tools

mcp_tools:
  - codegrep_search      # Fast code search with ripgrep
  - struct_find_replace  # Structural code editing with Comby
  - syntax_tree          # AST parsing with tree-sitter
  - test_run            # Multi-language test runner
  - sec_semgrep         # SAST security scanning
  - deps_osv            # Dependency vulnerability scanning
  - gh_checks           # GitHub Checks API
  - gh_pr_review        # PR review comments

## A2A Protocol Configuration

a2a:
  enabled: true
  discovery:
    mdns: true
    static_peers:
      - "claude-code-agent:8345"
      - "project-manager:8342"

  security:
    rate_limit: 100
    require_auth: false

  capabilities_broadcast:
    interval: 30
    include:
      - code_generation
      - code_search
      - security_scanning

## WebSocket Configuration
# WebSocket is used for `arkavo ui` connections, not agent-to-agent communication.
# Agents communicate via A2A protocol over HTTP.

websocket:
  enabled: true
  ping_interval: 30
  max_connections: 10

## Logging

logging:
  level: info
  file: logs/gemini-code-agent.log
  format: json

## Health Check

health_check:
  enabled: true
  interval: 60
  endpoint: /.well-known/agent.json

## Budget Tracking (Orchestrator Only)
# NOTE: Budget tracking is implemented in arkavo-budget crate and used by the orchestrator.
# Per-agent budget config from AGENTS.md is not yet parsed - use orchestrator-level config.
# For standalone agents, monitor usage via Google Cloud Console.

# budget:
#   enabled: true
#   limits:
#     hourly: 0.50
#     daily: 5.00
#     monthly: 50.00
#   alerts:
#     warning_percent: 80
#     critical_percent: 95

## Telemetry (Proposed - Not Yet Implemented)
# NOTE: OpenTelemetry integration is planned but not yet implemented.

# telemetry:
#   enabled: false
#   endpoint: http://localhost:4317
#   service_name: gemini-code-agent

## Environment Variable Overrides

# These can be set via environment variables:
# GEMINI_API_KEY - Google AI API key (required)
# GEMINI_MODEL - Override model selection
# GEMINI_BASE_URL - Alternative API endpoint (e.g., Vertex AI)

## Performance Tuning

performance:
  # Streaming optimizations
  streaming: true
  buffer_size: 8192

  # Tool execution
  tool_timeout: 30
  parallel_tools: 4

  # Rate limiting
  rate_limit:
    max_requests_per_minute: 60
    burst_size: 10

## Workspace Configuration

workspace:
  root: ./workspace
  max_size_mb: 1000
  auto_cleanup: true
  cleanup_after_hours: 24

## Code Generation Settings

code_generation:
  # Prefer Gemini's strengths
  preferred_frameworks:
    - react
    - vue
    - svelte
    - tailwind

  # Quality checks
  enforce_tests: true
  enforce_linting: true

  # File patterns
  allow_globs:
    - "**/*.js"
    - "**/*.ts"
    - "**/*.tsx"
    - "**/*.jsx"
    - "**/*.py"
    - "**/*.rs"
    - "**/*.go"
    - "**/*.java"
    - "**/*.html"
    - "**/*.css"
    - "**/*.scss"
    - "**/*.json"
    - "**/*.yaml"
    - "**/*.md"

  deny_globs:
    - "**/.env"
    - "**/.env.*"
    - "**/secrets/**"
    - "**/*.key"
    - "**/*.pem"
