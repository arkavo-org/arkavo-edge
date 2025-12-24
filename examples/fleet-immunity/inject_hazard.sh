#!/bin/bash
# Inject black ice hazard into Sector 4
#
# Sends HTTP request to the shared fleet environment server

FLEET_ENV_URL="${FLEET_ENV_URL:-http://localhost:8360}"

echo ""
echo "  ━━━━━━ INJECTING BLACK ICE INTO SECTOR 4 ━━━━━━"
echo ""

# Check if server is running
if ! curl -s "$FLEET_ENV_URL/health" > /dev/null 2>&1; then
    echo "[ERROR ] Fleet Environment server not running at $FLEET_ENV_URL"
    echo "[ERROR ] Start the fleet first with ./launch_fleet.sh"
    exit 1
fi

# Inject the hazard
RESPONSE=$(curl -s -X POST "$FLEET_ENV_URL/hazard" \
    -H "Content-Type: application/json" \
    -d '{"sector": 4, "hazard_type": "black_ice", "traction": 0.1}')

if echo "$RESPONSE" | grep -q '"success":true'; then
    echo "  [OK] Hazard injected successfully"
    echo ""
    echo "  [HAZARD] Black ice active in Sector 4 (Cold Storage)"
    echo "  [HAZARD] Traction coefficient: 0.1 (very slippery)"
    echo ""
    echo "  Watch for:"
    echo "    - Alpha enters Sector 4 → detects hazard → CRASH"
    echo "    - Alpha synthesizes safety lesson"
    echo "    - Beta/Gamma receive lesson via A2A"
    echo "    - Beta enters Sector 4 → SLOWS DOWN (no crash!)"
else
    echo "[ERROR ] Failed to inject hazard: $RESPONSE"
    exit 1
fi
echo ""
