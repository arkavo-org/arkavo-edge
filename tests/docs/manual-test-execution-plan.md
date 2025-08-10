# Manual Test Execution Plan - 79 Remaining Tests

## Overview
Systematic execution of 79 remaining tests organized into 8 phases with clear dependencies, resource requirements, and success criteria. Total estimated time: 16-20 hours.

## Test Status Summary
- ✅ **Completed**: 12 tests
- 🔄 **Remaining**: 79 tests
- 🎯 **Target Pass Rate**: ≥95% for release

## Phase 1: Foundation & Prerequisites (2 hours)
**Tests: 8 | Priority: Critical | Can be automated**

### Tests to Execute:
| Test ID | Test Name | Priority | Automation | Status |
|---------|-----------|----------|------------|--------|
| CLI-02 | Interactive chat mode verification | Critical | Partial | ⏳ Pending |
| CLI-04 | UI server startup | Critical | Yes | ⏳ Pending |
| ERR-01 | Invalid command handling | High | Yes | ⏳ Pending |
| ERR-02 | Offline execution handling | High | Yes | ⏳ Pending |
| DOC-01 | README instructions validation | Medium | Manual | ⏳ Pending |
| DOC-02 | Documentation generation | Medium | Yes | ⏳ Pending |
| REG-01 | Regression test suite | Critical | Yes | ⏳ Pending |
| CLAUDE-01 | CLAUDE.md file validity | High | Yes | ⏳ Pending |

### Prerequisites:
- Fix CLI-03 database issue (blocking agent tests)
- Ensure git repository is clean
- Verify network connectivity

### Test Commands:
```bash
# CLI-02: Interactive chat test
target/release/arkavo chat --prompt "Hello"

# CLI-04: UI server test
target/release/arkavo ui &
sleep 5
curl -I http://localhost:8080

# ERR-01: Invalid command
target/release/arkavo invalid-command 2>&1

# ERR-02: Offline test (requires network disconnection)
# Disconnect network, then:
target/release/arkavo chat --no-tui --prompt "Hello" 2>&1

# REG-01: Regression suite
.github/workflows/regression.yaml

# CLAUDE-01: Verify CLAUDE.md
test -f CLAUDE.md && grep -q "Project Overview" CLAUDE.md
```

---

## Phase 2: Agent Core Functionality (3 hours)
**Tests: 6 | Priority: Critical | Requires fixed CLI-03**

### Tests to Execute:
| Test ID | Test Name | Priority | Automation | Status |
|---------|-----------|----------|------------|--------|
| AGENT-01 | Conversational configuration | Critical | Manual | ⏳ Pending |
| AGENT-02 | Simple code modification | Critical | Partial | ⏳ Pending |
| AGENT-03 | File creation | High | Yes | ⏳ Pending |
| AGENT-04 | Interactive debugger | Medium | Manual | ⏳ Pending |
| AGENT-05 | Multi-agent knowledge sharing | High | Partial | ⏳ Pending |
| AGENT-06 | Cost budgeting layer | High | Yes | ⏳ Pending |

### Test Setup:
```bash
# Create test workspace
mkdir -p test-workspace
cd test-workspace

# Create sample Rust file for AGENT-02
cat > sample.rs << 'EOF'
fn main() {
    println!("Original code");
}
EOF

# Set budget for AGENT-06
export ARKAVO_BUDGET_LIMIT=0.10
```

---

## Phase 3: Git & GitHub Integration (2 hours)
**Tests: 3 | Priority: High | Depends on Agent tests**

### Tests to Execute:
| Test ID | Test Name | Priority | Automation | Status |
|---------|-----------|----------|------------|--------|
| GIT-01 | Auto-commit functionality | High | Yes | ⏳ Pending |
| GIT-02 | Feature branch creation | High | Yes | ⏳ Pending |
| GIT-03 | GitHub CLI integration | Medium | Partial | ⏳ Pending |

### Test Commands:
```bash
# GIT-01: Auto-commit test
# Make a change via agent, verify commit

# GIT-02: Branch creation
target/release/arkavo agent run --task "Create feature branch"

# GIT-03: GitHub CLI test
gh pr list
```

---

## Phase 4: LLM Provider Integration (4 hours)
**Tests: 15 | Priority: High | Can run in parallel**

### Tests to Execute:
| Test ID | Test Name | Priority | Automation | Status |
|---------|-----------|----------|------------|--------|
| LLM-01 | Kimi API integration | High | Yes | ⏳ Pending |
| LLM-02 | Model download manager | High | Yes | ⏳ Pending |
| LLM-03 | Local chat session | High | Yes | ⏳ Pending |
| LLM-04 | Ollama integration | Medium | Yes | ⏳ Pending |
| LLM-05 | Provider switching | High | Yes | ⏳ Pending |
| LLM-06 | Vision/image input | Medium | Partial | ⏳ Pending |
| OPENAI-01 | OpenAI API integration | High | Yes | ⏳ Pending |
| OPENAI-02 | GPT-4 Turbo model | Medium | Yes | ⏳ Pending |
| OPENAI-03 | GPT-4o Vision | Medium | Partial | ⏳ Pending |
| OPENAI-04 | Streaming responses | High | Yes | ⏳ Pending |
| OPENAI-05 | Cost tracking | High | Yes | ⏳ Pending |
| OPENAI-06 | Model switching | High | Yes | ⏳ Pending |
| OPENAI-07 | Rate limiting | Medium | Yes | ⏳ Pending |
| OPENAI-08 | Authentication error | High | Yes | ⏳ Pending |

### Setup:
```bash
# Load API keys
source .test.env

# Download test model
target/release/arkavo model download tinyllama

# Start Ollama (if testing)
ollama serve &

# Prepare test image
curl -o test-image.png https://picsum.photos/200
```

---

## Phase 5: UI & TUI Testing (2 hours)
**Tests: 7 | Priority: Medium | Manual testing required**

### Tests to Execute:
| Test ID | Test Name | Priority | Automation | Status |
|---------|-----------|----------|------------|--------|
| UI-01 | Orchestration dashboard | Medium | Manual | ⏳ Pending |
| TUI-01 | TUI stress test | Medium | Partial | ⏳ Pending |
| TUI-02 | Contextual tool display | Low | Manual | ⏳ Pending |
| CHAT-01 | Bidirectional protocol | High | Partial | ⏳ Pending |
| CHAT-02 | Context persistence | High | Yes | ⏳ Pending |
| CHAT-03 | Tool integration | High | Yes | ⏳ Pending |
| DATA-01 | Dataflow orchestration | Low | Manual | ⏳ Pending |

---

## Phase 6: iOS & Mobile Bridge (2 hours)
**Tests: 4 | Priority: Low | macOS only**

### Tests to Execute:
| Test ID | Test Name | Priority | Automation | Status |
|---------|-----------|----------|------------|--------|
| IOS-01 | iOS setup script | Low | Yes | ⏳ Pending |
| IOS-02 | iOS test execution | Low | Partial | ⏳ Pending |
| IOS-03 | Invalid simctl commands | Medium | Yes | ⏳ Pending |
| IOS-04 | Advanced test harness | Low | Partial | ⏳ Pending |

### macOS-specific Commands:
```bash
# IOS-01: Setup script
cd ios && sh setup_ios_bridge.sh

# IOS-02: Simulator test
xcrun simctl list devices

# IOS-03: Verify no invalid commands
# Monitor agent logs for "simctl tap" or "simctl swipe"
```

---

## Phase 7: Infrastructure & Security (2 hours)
**Tests: 7 | Priority: High | Some require special setup**

### Tests to Execute:
| Test ID | Test Name | Priority | Automation | Status |
|---------|-----------|----------|------------|--------|
| MCP-01 | MCP server functionality | High | Yes | ⏳ Pending |
| MCP-02 | MCP tool discovery | High | Yes | ⏳ Pending |
| MEM-01 | Memory lifecycle (1hr wait) | Medium | Yes | ⏳ Pending |
| MEM-02 | Maximum events per session | Medium | Yes | ⏳ Pending |
| SEC-01 | mTLS and OpenTDF auth | High | Partial | ⏳ Pending |
| WS-01 | WebSocket connections | High | Yes | ⏳ Pending |
| BUDGET-01 | Cost tracking and limits | High | Yes | ⏳ Pending |

### Test Commands:
```bash
# MCP-01: Server test
target/release/arkavo serve &
sleep 5
# Connect with MCP client

# MEM-01: Memory retention (requires 1-hour wait)
export ARKAVO_EVENT_RETENTION_HOURS=1
# Generate events, wait, verify cleanup

# WS-01: WebSocket test
websocat ws://localhost:8080/ws
```

---

## Phase 8: Platform & Performance (3 hours)
**Tests: 6 | Priority: Critical | Platform-specific**

### Tests to Execute:
| Test ID | Test Name | Priority | Automation | Status |
|---------|-----------|----------|------------|--------|
| PLAT-01 | macOS-specific execution | Critical | Yes | ⏳ Pending |
| PLAT-02 | Linux x64 execution | Critical | Partial | ⏳ Pending |
| PLAT-03 | Linux aarch64 execution | Medium | Partial | ⏳ Pending |
| PLAT-04 | macOS notarization workflow | Low | Manual | ⏳ Pending |
| PERF-03 | Metal NPU performance | Medium | Partial | ⏳ Pending |
| A2A-01 | A2A protocol implementation | High | Yes | ⏳ Pending |

### Platform Tests:
```bash
# PLAT-01: macOS suite
uname -a
./run-macos-tests.sh

# PLAT-02: Linux x64 (Docker)
docker run --rm -v $(pwd):/app ubuntu:22.04 /app/target/release/arkavo --help

# PERF-03: Metal NPU (macOS only)
# Run with Activity Monitor open to observe GPU usage
target/release/arkavo chat --local-model --prompt "Complex calculation"

# A2A-01: Protocol test
# Start two agents and verify communication
```

---

## Test Execution Tracking

### Progress Dashboard:
```
Phase 1: [⏳⏳⏳⏳⏳⏳⏳⏳] 0/8 tests
Phase 2: [⏳⏳⏳⏳⏳⏳] 0/6 tests  
Phase 3: [⏳⏳⏳] 0/3 tests
Phase 4: [⏳⏳⏳⏳⏳⏳⏳⏳⏳⏳⏳⏳⏳⏳⏳] 0/15 tests
Phase 5: [⏳⏳⏳⏳⏳⏳⏳] 0/7 tests
Phase 6: [⏳⏳⏳⏳] 0/4 tests
Phase 7: [⏳⏳⏳⏳⏳⏳⏳] 0/7 tests
Phase 8: [⏳⏳⏳⏳⏳⏳] 0/6 tests

Overall: 12/91 tests completed (13%)
```

### Success Criteria:
- **Critical tests**: 100% pass required
- **High priority**: ≥95% pass required
- **Medium priority**: ≥85% pass required
- **Low priority**: ≥70% pass acceptable

### Issue Tracking:
Failed tests will be logged with:
- Test ID and name
- Failure reason
- Stack trace/error message
- Reproduction steps
- Suggested fix

---

## Automation Scripts

### test-runner.sh
```bash
#!/bin/bash
# Automated test runner for Arkavo Edge

RESULTS_FILE="test-results-$(date +%Y%m%d-%H%M%S).json"
BINARY="target/release/arkavo"

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo "Starting Arkavo Edge Test Suite"
echo "================================"

# Function to run test and record result
run_test() {
    local test_id=$1
    local test_name=$2
    local test_cmd=$3
    
    echo -n "Running $test_id: $test_name... "
    
    if eval "$test_cmd" > /tmp/test-$test_id.log 2>&1; then
        echo -e "${GREEN}✅ PASS${NC}"
        echo "{\"test_id\": \"$test_id\", \"result\": \"pass\"}" >> $RESULTS_FILE
        return 0
    else
        echo -e "${RED}❌ FAIL${NC}"
        echo "{\"test_id\": \"$test_id\", \"result\": \"fail\"}" >> $RESULTS_FILE
        return 1
    fi
}

# Phase 1 Tests
echo "Phase 1: Foundation Tests"
run_test "CLI-04" "UI Server" "timeout 5 $BINARY ui"
run_test "ERR-01" "Invalid Command" "$BINARY invalid-cmd 2>&1 | grep -q 'help'"
run_test "DOC-02" "Documentation" "cargo doc --no-deps"

# Continue with more tests...

echo "Test execution complete. Results saved to $RESULTS_FILE"
```

---

## Next Steps

1. **Immediate Actions**:
   - Fix CLI-03 database issue
   - Set up test environment
   - Configure API keys

2. **Execution Order**:
   - Start with Phase 1 (Foundation)
   - Fix any blockers before Phase 2
   - Run Phases 4-7 in parallel where possible
   - Complete Phase 8 last (platform-specific)

3. **Documentation**:
   - Update test-results.md after each phase
   - Create GitHub issues for failures
   - Document workarounds for known issues

4. **Release Readiness**:
   - All critical tests must pass
   - Document any accepted failures
   - Create release notes with test summary