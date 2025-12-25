# AGENTS.md

## minecraft-router
purpose: "Thompson Sampling agent selector for Minecraft survival. Routes queries to optimal specialist: Scout (navigation/threats), Builder (resources/construction), Runner (objectives/escape)."
model:
listen: 0.0.0.0:8402
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
    - "http://localhost:8410"
    - "http://localhost:8411"
    - "http://localhost:8412"
