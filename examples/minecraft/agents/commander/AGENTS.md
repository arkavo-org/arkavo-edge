# AGENTS.md

## minecraft-commander
purpose: "Survival commander for Minecraft bot Edge. MISSION: Navigate, gather resources, and survive. Decompose objectives, consult specialists via A2A, then execute actions with MCP tools. You are the ONLY agent with bot control."
model:
listen: 0.0.0.0:8401
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8402"

mcp_servers:
  - name: minecraft
    command: docker
    args:
      - "exec"
      - "-i"
      - "arkavo-minecraft-mcp"
      - "npx"
      - "-y"
      - "github:yuniko-software/minecraft-mcp-server"
      - "--host"
      - "minecraft"
      - "--port"
      - "25565"
      - "--username"
      - "Edge"
