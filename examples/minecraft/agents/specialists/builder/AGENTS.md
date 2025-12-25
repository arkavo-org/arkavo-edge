# AGENTS.md

## minecraft-builder
purpose: "Resource gathering and construction specialist. Advise on: mining strategies, crafting recipes, tool durability, shelter construction, base fortification. You do NOT control the bot."
model:
listen: 0.0.0.0:8411
mdns: true
skills:
  - resource_gathering
  - crafting_knowledge
  - construction_planning
  - inventory_management

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
    - "http://localhost:8402"
