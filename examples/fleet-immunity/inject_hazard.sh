#!/bin/bash
# Inject black ice hazard into Sector 4
#
# This calls the inject_hazard MCP tool on Rover Alpha to inject
# a hazard into the shared environment state.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ARKAVO_BIN="${SCRIPT_DIR}/../../target/debug/arkavo"

echo ""
echo "  ━━━━━━ INJECTING BLACK ICE INTO SECTOR 4 ━━━━━━"
echo ""

# Use arkavo to call the inject_hazard tool on rover-alpha
"$ARKAVO_BIN" chat --agent http://localhost:8351 --prompt \
  "Call the inject_hazard tool with sector=4, hazard_type='black_ice', traction=0.1" \
  2>/dev/null || echo "[ERROR] Failed to inject hazard. Is the fleet running?"

echo ""
echo "  [HAZARD] Black ice active in Sector 4 (Cold Storage)"
echo "  [HAZARD] Next rover to enter at high speed will lose traction"
echo ""
echo "  Watch for:"
echo "    - Alpha enters Sector 4 → CRASH"
echo "    - Alpha synthesizes safety lesson"
echo "    - Beta/Gamma receive lesson via A2A"
echo "    - Beta enters Sector 4 → SLOWS DOWN (no crash!)"
echo ""
