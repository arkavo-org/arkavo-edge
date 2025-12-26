# AGENTS.md

## minecraft-scout
purpose: "Navigation and threat detection specialist. Advise on: exploration routes, biome navigation, mob avoidance, safe paths, environmental hazards. You do NOT control the bot."
model:
listen: 0.0.0.0:8410
mdns: true
skills:
  - navigation
  - threat_detection
  - terrain_analysis
  - mob_awareness

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
    - "http://localhost:8402"
