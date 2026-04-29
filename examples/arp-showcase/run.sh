#!/bin/bash
# ARP Showcase — boot the AG-UI web gateway with an Agent Runtime Policy
# document loaded so the new ARP panel has data to display.
#
# Usage:
#   ./run.sh           # default port 7700
#   ./run.sh --port N  # custom port
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

echo "ARP Showcase"
echo "============"
echo "ARP document: $ARKAVO_ARP_PATH"
echo "Web UI port:  $PORT"
echo
echo "Open http://127.0.0.1:${PORT} and click the scales (Agent Runtime Policy) nav button."
echo

exec "$BINARY" ui --port "$PORT"
