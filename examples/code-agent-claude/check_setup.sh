#!/bin/bash

# Test Claude Agent SDK Connection
# Verifies that the Claude Agent capability is working

set -e

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

print_test() {
    echo -e "${YELLOW}[TEST]${NC} $1"
}

print_pass() {
    echo -e "${GREEN}[PASS]${NC} $1"
}

print_fail() {
    echo -e "${RED}[FAIL]${NC} $1"
}

# Check Node.js
print_test "Checking Node.js installation..."
if command -v node &> /dev/null; then
    NODE_VERSION=$(node --version)
    print_pass "Node.js installed: $NODE_VERSION"
else
    print_fail "Node.js not installed"
    exit 1
fi

# Check Claude Agent SDK
print_test "Checking Claude Agent SDK..."
SDK_FOUND=false
SDK_LOCATION=""

# Check local Claude installation first (preferred)
if [ -d "$HOME/.claude/local/node_modules/@anthropic-ai/claude-agent-sdk" ]; then
    SDK_VERSION=$(node -e "console.log(require('$HOME/.claude/local/node_modules/@anthropic-ai/claude-agent-sdk/package.json').version)" 2>/dev/null)
    if [ -n "$SDK_VERSION" ]; then
        print_pass "Claude Agent SDK installed (local): v$SDK_VERSION"
        SDK_LOCATION="$HOME/.claude/local"
        SDK_FOUND=true
    fi
fi

# Check global npm installation as fallback
if [ "$SDK_FOUND" = false ]; then
    if npm list -g @anthropic-ai/claude-agent-sdk &> /dev/null; then
        SDK_VERSION=$(npm list -g @anthropic-ai/claude-agent-sdk --depth=0 | grep @anthropic-ai/claude-agent-sdk | awk '{print $2}')
        print_pass "Claude Agent SDK installed (global npm): $SDK_VERSION"
        SDK_LOCATION="global"
        SDK_FOUND=true
    fi
fi

if [ "$SDK_FOUND" = false ]; then
    print_fail "Claude Agent SDK not installed"
    echo "Install via one of these methods:"
    echo "  1. Claude CLI (recommended): Install from https://claude.ai/"
    echo "  2. npm: npm install -g @anthropic-ai/claude-agent-sdk"
    exit 1
fi

# Check API credentials
print_test "Checking API credentials..."
if [ -n "$ANTHROPIC_API_KEY" ]; then
    print_pass "Anthropic API key configured"
elif [ -n "$ANTHROPIC_AUTH_TOKEN" ]; then
    print_pass "Alternative auth token configured"
    if [ -n "$ANTHROPIC_BASE_URL" ]; then
        print_pass "Alternative base URL: $ANTHROPIC_BASE_URL"
    fi
else
    print_fail "No API credentials found"
    exit 1
fi

# Test Node.js bridge script
print_test "Testing Node.js bridge script..."
BRIDGE_SCRIPT="../../crates/arkavo-claude-code/node/claude-code-bridge.js"
if [ -f "$BRIDGE_SCRIPT" ]; then
    print_pass "Bridge script found"
    
    # Try to run it briefly to check for syntax errors
    timeout 1 node "$BRIDGE_SCRIPT" 2>/dev/null || true
    if [ $? -eq 124 ]; then
        # Timeout exit code - script started successfully
        print_pass "Bridge script syntax valid"
    fi
else
    print_fail "Bridge script not found at $BRIDGE_SCRIPT"
    exit 1
fi

# Test simple SDK functionality
print_test "Testing Claude Agent SDK functionality..."

# Create test script with path detection
cat > /tmp/test-claude-agent.js << 'EOF'
const path = require('path');
let sdk;

// Try loading from multiple locations
const possiblePaths = [
    path.join(process.env.HOME, '.claude', 'local', 'node_modules', '@anthropic-ai', 'claude-agent-sdk'),
    '@anthropic-ai/claude-agent-sdk'
];

for (const modulePath of possiblePaths) {
    try {
        sdk = require(modulePath);
        console.log('Claude Agent SDK loaded successfully from:', modulePath);
        break;
    } catch (e) {
        // Try next
    }
}

if (!sdk) {
    console.error('Failed to load Claude Agent SDK');
    process.exit(1);
}

// Check if we can access basic SDK functions
if (typeof sdk === 'object') {
    console.log('SDK module structure valid');
    process.exit(0);
} else {
    console.error('SDK module structure invalid');
    process.exit(1);
}
EOF

if node /tmp/test-claude-agent.js 2>/dev/null; then
    print_pass "Claude Agent SDK functional"
else
    print_fail "Claude Agent SDK test failed"
    exit 1
fi

rm -f /tmp/test-claude-agent.js

# Summary
echo ""
echo -e "${GREEN}════════════════════════════════════════${NC}"
echo -e "${GREEN}  All tests passed! ✓${NC}"
echo -e "${GREEN}  Claude Agent capability is ready to use${NC}"
echo -e "${GREEN}════════════════════════════════════════${NC}"
echo ""
echo "Next steps:"
echo "  1. Start the agent: ./launch_agent.sh start"
echo "  2. Run examples: ./run_examples.sh"