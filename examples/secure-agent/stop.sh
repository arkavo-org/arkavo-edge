#!/bin/bash
# Stop the secure-agent

pkill -f "secure-agent" 2>/dev/null || true
echo "Secure agent stopped"
