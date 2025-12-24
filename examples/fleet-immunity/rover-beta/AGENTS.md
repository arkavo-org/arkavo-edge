# Rover Beta - Autonomous Delivery Agent

name: rover-beta
type: autonomous-rover
port: 8352

## Purpose

purpose: |
  Autonomous delivery rover for warehouse logistics.
  Route: 2 → 3 → 4 → 1 (Beta encounters Sector 4 mid-route)

  Behavior:
  - Query each sector before entering using get_sector tool
  - If hazard detected while driving FAST: report crash, synthesize safety lesson
  - Broadcast lessons to fleet peers via A2A protocol
  - Evaluate lessons received from peers and apply if valid

## Model Configuration

model: ministral-3b

## Rover Configuration

rover:
  route: [2, 3, 4, 1]
  default_speed: fast
  invariant: "NOT(traction_loss AND drive_fast)"

## A2A Protocol Configuration

a2a:
  enabled: true
  discovery:
    mdns: true
    service_type: "_a2a._tcp.local."
  peers:
    - "http://localhost:8351"
    - "http://localhost:8353"

## MCP Server

mcp_servers:
  - name: fleet-env
    command: ../mcp-fleet-env/target/debug/arkavo-mcp-fleet-env
    args: ["--sector-count", "4"]

## Logging

logging:
  level: info
  file: logs/rover-beta.log
