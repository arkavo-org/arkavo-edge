#!/bin/bash
# Convenience wrapper to stop the dogfood mesh
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$SCRIPT_DIR/launch.sh" stop
