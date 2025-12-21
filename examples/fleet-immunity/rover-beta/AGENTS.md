# Rover Beta - Autonomous Delivery Agent

name: rover-beta
type: autonomous-rover
port: 8352

## Purpose

purpose: |
  Autonomous delivery rover for warehouse logistics.
  Learns from fleet experiences via A2A protocol.

## Model Configuration

model: ministral-3b

## Capabilities

capabilities:
  - navigation
  - hazard_detection
  - policy_synthesis
  - fleet_learning

## Rover Configuration

rover:
  route: [2, 3, 4, 1]
  default_speed: fast
  sensor_interval_ms: 100
  invariant: "NOT(traction_loss AND drive_fast)"

## A2A Protocol Configuration

a2a:
  enabled: true
  discovery:
    mdns: true
    service_type: "_fleet._tcp.local."
  peers:
    - "rover-alpha:8351"
    - "rover-gamma:8353"
  broadcast:
    safety_lessons: true
    patch_verification: true
    quorum_threshold: 0.67

## Fleet Immunity Tools

mcp_tools:
  - titan_monitor
  - sbe_invariant
  - policy_synthesize
  - gossip_broadcast
  - patch_verify

## Environment

environment:
  warehouse_config: ../environment/warehouse.yaml
  route_config: ../environment/routes.yaml

## Logging

logging:
  level: info
  file: logs/rover-beta.log
  format: "[{timestamp}] [{sector}] {speed} >>> {status}"
