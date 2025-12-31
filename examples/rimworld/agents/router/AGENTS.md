# AGENTS.md

## rimworld-router
purpose: |
  Thompson Sampling agent selector for RimWorld colony management.
  Routes queries to optimal specialist based on domain:

  - Survival (port 8410): Food, hunger, health, mood, temperature, exhaustion
  - Industry (port 8411): Work priorities, mining, construction, farming, production
  - Defense (port 8412): Combat, raids, threats, drafting, fortification

  Analyze the query and select the most appropriate specialist.
  Track success/failure of specialist advice to improve selection over time.

model: mistralai/Ministral-3-3B-Instruct-2512-GGUF
listen: 0.0.0.0:8402
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
    - "http://localhost:8410"
    - "http://localhost:8411"
    - "http://localhost:8412"
