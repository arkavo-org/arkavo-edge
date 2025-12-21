#!/bin/bash
# Inject black ice hazard into Sector 4

echo ""
echo "  ━━━━━━ INJECTING BLACK ICE INTO SECTOR 4 ━━━━━━"
echo ""

# Activate hazard in environment
curl -s -X POST http://localhost:8360/inject \
  -H "Content-Type: application/json" \
  -d '{
    "sector": 4,
    "hazard": "black_ice",
    "traction": 0.1
  }' || echo "[ERROR] Failed to inject hazard. Is the environment running?"

echo ""
echo "  [HAZARD] Black ice active in Sector 4 (Cold Storage)"
echo "  [HAZARD] Next rover to enter at high speed will lose traction"
echo ""
echo "  Watch for:"
echo "    - Alpha enters Sector 4 -> CRASH"
echo "    - Alpha synthesizes safety lesson"
echo "    - Beta/Gamma receive lesson via A2A"
echo "    - Beta enters Sector 4 -> SLOWS DOWN (no crash!)"
echo ""
