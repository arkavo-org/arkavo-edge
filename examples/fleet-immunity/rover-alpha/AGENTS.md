# Rover Alpha - Autonomous Delivery Agent

name: rover-alpha
type: autonomous-rover
port: 8351

## Purpose

purpose: |
  Autonomous delivery rover for warehouse logistics.
  Navigates sectors, detects hazards, learns from crashes,
  and shares safety lessons with the fleet.

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
  route: [1, 2, 4, 3]
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
    - "rover-beta:8352"
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
  file: logs/rover-alpha.log
  format: "[{timestamp}] [{sector}] {speed} >>> {status}"
