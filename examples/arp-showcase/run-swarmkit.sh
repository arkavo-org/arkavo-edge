#!/bin/bash
# ARP Showcase + SwarmKit — boot the AG-UI gateway with BOTH:
#   * the rich (local) ARP document (arkavo.arp.json), and
#   * a single-role SwarmFlight derived from arkavo.swarmkit.yaml
# so the panel renders the original ARP showcase agent alongside an
# Active SwarmFlight section, demonstrating the SwarmKit per-role ARP
# handoff in the same UI.
#
# Usage:
#   ./run-swarmkit.sh           # default port 7700
#   ./run-swarmkit.sh --port N  # custom port
#
# Open http://127.0.0.1:7700 then click the scales icon in the left nav.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="${SCRIPT_DIR}/../../target/debug/arkavo"

PORT=7700
while [[ $# -gt 0 ]]; do
    case "$1" in
        --port) PORT="$2"; shift 2 ;;
        *) shift ;;
    esac
done

if [ ! -f "$BINARY" ]; then
    echo "Error: arkavo binary not found at $BINARY"
    echo "Build first:  cargo build"
    exit 1
fi

export ARKAVO_ARP_PATH="${SCRIPT_DIR}/arkavo.arp.json"
export ARKAVO_SWARMKIT_PATH="${SCRIPT_DIR}/arkavo.swarmkit.yaml"

echo "ARP Showcase + SwarmKit"
echo "======================="
echo "ARP document: $ARKAVO_ARP_PATH"
echo "SwarmKit:     $ARKAVO_SWARMKIT_PATH"
echo "Web UI port:  $PORT"
echo
echo "Open http://127.0.0.1:${PORT} and click the scales (Agent Runtime Policy) nav button."
echo "You should see an 'Active SwarmFlights' section above the (local) agent."
echo

exec "$BINARY" ui --port "$PORT"
