# Minecraft MCP Bot Runbook

This runbook describes how to run and validate the Minecraft MCP integration.

## Overview

This example demonstrates:
- MCP stdio transport for subprocess communication
- Real-time game bot control via tool calls
- Natural language to game action translation

## Prerequisites

```bash
# Build Arkavo
cargo build -p arkavo

# Verify Docker is running
docker info

# Verify Node.js version (20+ required)
node --version
```

## Running the Demo

### Step 1: Start Minecraft Server

```bash
docker compose up -d minecraft
```

**What to watch for:**
- Container starts without errors
- Server initializes world generation

```bash
# Monitor startup (wait for "Done" message)
docker compose logs -f minecraft
```

**Expected output:**
```
[Server] Starting minecraft server version 1.21.8
[Server] Preparing level "world"
[Server] Done (12.345s)! For help, type "help"
```

### Step 2: Verify Server Ready

```bash
# Check container is running
docker compose ps

# Test port is accessible
nc -zv localhost 25565
```

**Expected:** Connection succeeds.

### Step 3: Launch the Agent

```bash
./launch_minecraft.sh
```

**What to watch for:**
- MCP server spawns successfully
- Bot connects to Minecraft server
- Tools are discovered and registered

**Expected output:**
```
[MINECRAFT] Starting Minecraft agent...
[MCP      ] Spawning minecraft-mcp-server...
[MCP      ] Connected, discovering tools...
[MCP      ] Registered 8 tools from minecraft server
[MINECRAFT] Bot "ClaudeBot" joined the game
```

### Step 4: Interact with the Bot

The agent accepts natural language commands. Try:

```
> Look around and tell me what you see
> Move forward 10 blocks
> Mine the nearest tree
> What's in your inventory?
> Build a small shelter
```

### Step 5: Watch in Minecraft Client (Optional)

1. Open Minecraft Java Edition 1.21.x
2. Multiplayer > Direct Connect > `localhost:25565`
3. Join and find ClaudeBot

### Step 6: Stop Everything

```bash
./stop_minecraft.sh
```

## Verification

### Manual Test

```bash
# 1. Start server
docker compose up -d minecraft

# 2. Wait for ready
sleep 30

# 3. Test MCP server directly
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test"}}}' | \
  npx -y github:yuniko-software/minecraft-mcp-server \
    --host localhost --port 25565 --username TestBot

# 4. Stop
docker compose down
```

### Expected MCP Response

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2024-11-05",
    "serverInfo": {"name": "minecraft-mcp-server"},
    "capabilities": {"tools": {}}
  }
}
```

## Troubleshooting

### Server Not Starting

```bash
# Check Docker logs
docker compose logs minecraft

# Common issue: EULA not accepted (compose.yaml sets EULA=TRUE)
# Common issue: Port 25565 already in use
lsof -i :25565
```

### Bot Not Connecting

```bash
# Verify server accepts connections
nc -zv localhost 25565

# Check MCP server logs
# The bot needs ~5 seconds to spawn and connect
```

### MCP Server Fails

```bash
# Verify Node.js version
node --version  # Must be 20+

# Try running MCP server directly
npx -y github:yuniko-software/minecraft-mcp-server --help
```

### Tools Not Working

```bash
# Check if bot has spawned in-game
# Some tools require the bot to be fully loaded

# Verify with get_position first
```

## Architecture Notes

### Why Stdio Transport?

The MCP specification defines stdio as the primary transport for local tool servers. The client (Arkavo) spawns the server as a subprocess and communicates via stdin/stdout with JSON-RPC messages.

### Why Docker for Minecraft?

- Consistent server version
- No manual Java/server setup
- Easy cleanup with `docker compose down -v`
- Offline mode for simpler testing (no Mojang auth)

### Bot Limitations

- Single bot per MCP server instance
- Bot respawns if killed (Mineflayer behavior)
- Some complex actions may require multiple tool calls

## Related Files

- `AGENTS.md` - Agent configuration with MCP server definition
- `compose.yaml` - Docker Compose for Minecraft server
- `logs/` - Runtime logs (gitignored)
