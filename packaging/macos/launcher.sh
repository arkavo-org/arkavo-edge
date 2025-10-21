#!/bin/bash
# Arkavo .app launcher script
# Launches the Arkavo UI when the app bundle is opened

# Get the directory where this script is located
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# Change to user's home directory for a sensible working directory
cd "$HOME" || exit 1

# Launch the Arkavo UI
exec "$SCRIPT_DIR/arkavo-bin" ui
