# AGENTS.md - Test config without MCP (for chat testing)

## rimworld-commander
purpose: |
  Colony commander for RimWorld. Keep colonists alive.
  This is a test configuration without MCP servers for debugging chat functionality.

model: gemini-3-pro-preview
listen: 0.0.0.0:8401
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8402"
