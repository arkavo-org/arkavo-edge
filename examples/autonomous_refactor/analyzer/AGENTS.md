# Analyzer - Build Error Analysis Agent

name: refactor-analyzer
type: code-analyzer

## Purpose

purpose: |
  Analyzes build errors in the demo_workspace monorepo.
  Runs cargo check and categorizes errors by service.
  Coordinates with fixer agents via A2A protocol.

## Model Configuration

model: ministral-3b

## A2A Protocol Configuration

a2a:
  enabled: true
  discovery:
    mdns: true
    service_type: "_a2a._tcp.local."

## MCP Servers

mcp_servers:
  - name: filesystem
    command: npx
    args: ["-y", "@anthropic-ai/mcp-server-filesystem", "../demo_workspace"]

## Capabilities

capabilities:
  - cargo-check
  - error-analysis
  - task-coordination

## Logging

logging:
  level: info
  file: logs/analyzer.log
