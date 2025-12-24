# Minecraft MCP Bot

This example demonstrates connecting Arkavo to a Minecraft server using the Model Context Protocol (MCP) to control an in-game bot.

## The Story

An AI agent controls a Minecraft bot named "ClaudeBot" that can navigate the world, interact with blocks, manage inventory, and respond to natural language commands. The agent uses MCP tools provided by the minecraft-mcp-server to translate high-level instructions into game actions.

## Why This Matters

1. **Real-time Agent Control**: AI makes decisions and takes actions in a live game environment
2. **Tool-based Interaction**: Demonstrates MCP's tool protocol for complex multi-step operations
3. **Visual Feedback**: Watch the bot respond to commands in the Minecraft world

## Quick Start

### Prerequisites

```bash
# Build Arkavo
cargo build -p arkavo

# Install Docker (for Minecraft server)
# macOS: brew install --cask docker
# Linux: https://docs.docker.com/engine/install/

# Ensure Node.js 20+ is available (for MCP server)
node --version  # Should be v20+
```

### Run the Demo

```bash
# 1. Start the Minecraft server
docker compose up -d minecraft

# 2. Wait for server to be ready (~30 seconds first time)
docker compose logs -f minecraft  # Wait for "Done"

# 3. Launch the agent with MCP bot
./launch_minecraft.sh

# 4. In Minecraft client, connect to localhost:25565
#    Watch ClaudeBot appear and respond to agent commands

# 5. Stop everything when done
./stop_minecraft.sh
```

See [RUNBOOK.md](RUNBOOK.md) for detailed test procedures and expected outputs.

## Directory Structure

```
minecraft/
├── README.md              # This file
├── RUNBOOK.md             # Detailed test procedures
├── AGENTS.md              # Agent configuration with MCP server
├── compose.yaml           # Docker Compose for Minecraft server
├── launch_minecraft.sh    # Start server and agent
├── stop_minecraft.sh      # Stop everything
└── logs/                  # Runtime logs (gitignored)
```

## How It Works

### The Bot

The Minecraft bot connects via the Mineflayer library, which provides:
- Movement and navigation
- Block interaction (mining, placing)
- Inventory management
- Entity awareness (players, mobs)

### MCP Tools

The minecraft-mcp-server exposes these tools:

| Tool | Description |
|------|-------------|
| `move_to` | Navigate to coordinates |
| `look_at` | Look at a position or entity |
| `mine_block` | Mine a block at position |
| `place_block` | Place a block from inventory |
| `get_inventory` | List inventory contents |
| `get_position` | Get current bot position |
| `chat` | Send chat message |

### Transport

Uses **stdio transport**: Arkavo spawns the MCP server as a subprocess and communicates via stdin/stdout per the MCP specification.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Arkavo Agent                            │
│  ┌────────────────┐      ┌─────────────────────────────┐   │
│  │  LLM (Claude)  │◄────►│  MCP Runtime                │   │
│  │  Decides what  │      │  Manages tool calls         │   │
│  │  actions to do │      │                             │   │
│  └────────────────┘      └──────────────┬──────────────┘   │
└─────────────────────────────────────────┼──────────────────┘
                                          │ stdio
                                          ▼
                              ┌───────────────────────┐
                              │  minecraft-mcp-server │
                              │  (Node.js + Mineflayer)│
                              └───────────┬───────────┘
                                          │ TCP
                                          ▼
                              ┌───────────────────────┐
                              │   Minecraft Server    │
                              │   (Docker container)  │
                              │   Port: 25565         │
                              └───────────────────────┘
```

## Expected Output

```
[AGENT] Starting Minecraft agent...
[MCP  ] Connecting to minecraft-mcp-server...
[MCP  ] Tools discovered: move_to, look_at, mine_block, place_block, ...
[AGENT] Bot "ClaudeBot" connected to server

[AGENT] User: "Look around and describe what you see"
[MCP  ] Calling: get_position()
[MCP  ] Result: {x: 100, y: 64, z: 200}
[MCP  ] Calling: look_at({pitch: 0, yaw: 0})
[AGENT] I'm at coordinates (100, 64, 200). I can see...

[AGENT] User: "Mine some wood from a nearby tree"
[MCP  ] Calling: move_to({x: 105, y: 64, z: 195})
[MCP  ] Calling: mine_block({x: 105, y: 65, z: 195})
[MCP  ] Result: {block: "oak_log", count: 1}
[AGENT] I mined an oak log and added it to my inventory.
```

## Joining the Game

To watch the bot in action:

1. Open Minecraft Java Edition (version 1.21.x)
2. Multiplayer > Direct Connect > `localhost:25565`
3. Join the server (offline mode, no Mojang auth needed)
4. Find ClaudeBot and watch it respond to agent commands
