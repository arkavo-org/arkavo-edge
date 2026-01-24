# Claude Code Agent Configuration

Example configuration for an agent using the native Claude Agent SDK.

name: claude-code-agent
type: coding

## Claude Code Configuration

claude_code:
  enabled: true
  use_oauth: true
  workspace_root: ./workspace
  budget_tokens: 200000

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

## Authentication

# OAuth is used by default for Claude Max/Pro subscribers.
# To use an API key instead, set the environment variable:
#   export ANTHROPIC_API_KEY="sk-ant-..."
