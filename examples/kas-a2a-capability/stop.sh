#!/bin/bash
# Stop the KAS agent
pkill -f "arkavo agent run" || true
echo "KAS agent stopped"
