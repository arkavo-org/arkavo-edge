# Claude Code Agent Configuration

Agent using the native Claude Agent SDK with bidirectional sessions,
budget tracking, and MCP tool integration.

name: claude-code-agent
type: coding

## Claude Code Configuration

claude_code:
  enabled: true
  use_oauth: true
  use_bidirectional: true
  workspace_root: ./workspace
  budget_tokens: 200000
  max_budget_usd: 5.0
  permission_mode: default

  tools:
    read: true
    write: true
    exec: false
    web_search: true

  allow_globs:
    - "**/*.rs"
    - "**/*.py"
    - "**/*.js"
    - "**/*.ts"
    - "**/*.md"
    - "**/*.json"
    - "**/*.toml"

  deny_globs:
    - "**/.env"
    - "**/secrets/**"
    - "**/node_modules/**"
    - "**/target/**"

  allowed_tools:
    - Read
    - Write
    - Edit
    - Glob
    - Grep
    - WebFetch
    - WebSearch

  disallowed_tools:
    - Bash

## Authentication

# OAuth is used by default for Claude Max/Pro subscribers.
# To use an API key instead, set the environment variable:
#   export ANTHROPIC_API_KEY="sk-ant-..."
