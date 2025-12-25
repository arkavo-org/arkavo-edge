# Fixer Gamma - Service C Repair Agent

name: fixer-gamma
type: code-fixer

## Purpose

purpose: |
  Fixes build errors in service_c of the demo_workspace.
  Fix pattern: replace process_id(100) with process_id(100.to_string())

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
  - code-edit
  - service-c

## Logging

logging:
  level: info
  file: logs/fixer-gamma.log
