# Manual Test Execution Checklist

## Overview
This checklist covers the 79 remaining tests that require manual verification or special setup. Check off each test as completed and note the result.

## Pre-Test Setup

### Environment Preparation
- [ ] Build release binary: `cargo build --release`
- [ ] Set up test workspace: `mkdir -p test-workspace`
- [ ] Configure API keys in `.test.env`
- [ ] Install required tools: `gh`, `ollama`, `websocat`
- [ ] Verify network connectivity
- [ ] Clean git working directory

### Test Data
- [ ] Create sample code files in `test-workspace/`
- [ ] Download test image: `curl -o test-image.png https://picsum.photos/200`
- [ ] Prepare test prompts document

---

## Phase 1: Foundation & Prerequisites ✅ 

### CLI Tests
- [ ] **CLI-02**: Interactive Chat Mode
  ```bash
  target/release/arkavo chat --prompt "Hello"
  ```
  - [ ] TUI launches successfully
  - [ ] Model loads correctly
  - [ ] Response displayed
  - [ ] Keyboard navigation works
  - Result: ________

- [ ] **CLI-04**: UI Server
  ```bash
  target/release/arkavo ui
  ```
  - [ ] Server starts on correct port
  - [ ] Web UI accessible in browser
  - [ ] No console errors
  - Result: ________

### Error Handling
- [ ] **ERR-01**: Invalid Command
  ```bash
  target/release/arkavo invalid-command
  ```
  - [ ] User-friendly error message
  - [ ] Suggests --help
  - [ ] No panic/crash
  - Result: ________

- [ ] **ERR-02**: Offline Execution
  - [ ] Disconnect network
  - [ ] Run: `target/release/arkavo chat --prompt "Hello"`
  - [ ] Graceful error message
  - [ ] No hang/crash
  - Result: ________

---

## Phase 2: Agent Core Functionality 🤖

### Agent Configuration
- [ ] **AGENT-01**: Conversational Configuration
  - [ ] Start agent requiring branch name
  - [ ] Agent asks clear question
  - [ ] Accepts user input
  - [ ] Creates branch correctly
  - Result: ________

- [ ] **AGENT-02**: Code Modification
  ```bash
  # Create test file first
  echo 'fn main() { }' > test.rs
  # Ask agent to modify
  target/release/arkavo chat --prompt "Add a println to test.rs"
  ```
  - [ ] Identifies correct file
  - [ ] Makes valid modification
  - [ ] Code compiles after change
  - Result: ________

- [ ] **AGENT-03**: File Creation
  - [ ] Ask: "Create docs/test.md with 'Hello World'"
  - [ ] File created at correct path
  - [ ] Content matches request
  - Result: ________

### Advanced Agent Features
- [ ] **AGENT-04**: Interactive Debugger
  - [ ] Enable debugger mode
  - [ ] Set breakpoint
  - [ ] Step through execution
  - [ ] Inspect variables
  - Result: ________

- [ ] **AGENT-05**: Multi-Agent Knowledge Sharing
  - [ ] Launch 2 agents
  - [ ] Agent A learns information
  - [ ] Agent B queries Agent A
  - [ ] Information transferred correctly
  - Result: ________

- [ ] **AGENT-06**: Budget Control
  - [ ] Set budget: `export ARKAVO_BUDGET_LIMIT=0.01`
  - [ ] Run expensive task
  - [ ] Warning near limit
  - [ ] Stops at limit
  - Result: ________

---

## Phase 3: Git Integration 📝

- [ ] **GIT-01**: Auto-commit
  - [ ] Make file change via agent
  - [ ] Check `git status`
  - [ ] Verify commit created
  - [ ] Commit message appropriate
  - Result: ________

- [ ] **GIT-02**: Branch Creation
  - [ ] Request new feature
  - [ ] Provide branch name when asked
  - [ ] Verify with `git branch`
  - Result: ________

- [ ] **GIT-03**: GitHub CLI
  - [ ] Ask: "List open PRs"
  - [ ] Agent uses `gh pr list`
  - [ ] Results displayed correctly
  - Result: ________

---

## Phase 4: LLM Providers 🧠

### Local Models
- [ ] **LLM-01**: Kimi API
  - [ ] Configure Kimi credentials
  - [ ] Run chat with Kimi model
  - [ ] Response from Kimi
  - Result: ________

- [ ] **LLM-02**: Model Download
  ```bash
  target/release/arkavo model download tinyllama
  ```
  - [ ] Download progress shown
  - [ ] Model saved correctly
  - [ ] Can list downloaded models
  - Result: ________

- [ ] **LLM-03**: Local Chat
  - [ ] Use downloaded model
  - [ ] No external API calls
  - [ ] Reasonable performance
  - Result: ________

- [ ] **LLM-04**: Ollama Integration
  - [ ] Start Ollama: `ollama serve`
  - [ ] Connect Arkavo to Ollama
  - [ ] List available models
  - [ ] Chat with Ollama model
  - Result: ________

### OpenAI Tests
- [ ] **OPENAI-01**: Basic Integration
  - [ ] Set OPENAI_API_KEY
  - [ ] Chat with GPT-3.5-turbo
  - [ ] Response received
  - Result: ________

- [ ] **OPENAI-02**: GPT-4 Turbo
  - [ ] Switch to GPT-4-turbo
  - [ ] Complex query works
  - Result: ________

- [ ] **OPENAI-03**: Vision (GPT-4o)
  - [ ] Upload image
  - [ ] Ask about image content
  - [ ] Accurate description
  - Result: ________

- [ ] **OPENAI-04**: Streaming
  - [ ] Enable streaming mode
  - [ ] Tokens appear progressively
  - Result: ________

- [ ] **OPENAI-05**: Cost Tracking
  - [ ] Enable budget tracking
  - [ ] Run multiple queries
  - [ ] View cost report
  - [ ] Costs calculated correctly
  - Result: ________

---

## Phase 5: UI & TUI 🖥️

### Terminal UI
- [ ] **TUI-01**: Stress Test
  - [ ] Start chat
  - [ ] Rapidly resize terminal
  - [ ] Paste large text block
  - [ ] Random key combinations
  - [ ] No crash/hang
  - Result: ________

- [ ] **TUI-02**: Tool Display
  - [ ] Start agent task
  - [ ] Observe tool list
  - [ ] Context-appropriate tools shown
  - Result: ________

### Chat Features
- [ ] **CHAT-01**: Bidirectional Protocol
  - [ ] Ask ambiguous question
  - [ ] Agent asks for clarification
  - [ ] Provide clarification
  - [ ] Agent proceeds correctly
  - Result: ________

- [ ] **CHAT-02**: Context Persistence
  - [ ] Chat: "My name is Alice"
  - [ ] Close chat
  - [ ] Reopen chat
  - [ ] Ask: "What's my name?"
  - [ ] Should answer "Alice"
  - Result: ________

- [ ] **CHAT-03**: Tool Integration
  - [ ] Ask: "Create file test.txt"
  - [ ] Agent identifies write_file tool
  - [ ] File created successfully
  - Result: ________

### Web UI
- [ ] **UI-01**: Dashboard
  - [ ] Start UI server
  - [ ] Open in browser
  - [ ] Start/stop agent from UI
  - [ ] Status updates in realtime
  - Result: ________

- [ ] **DATA-01**: Dataflow Visualization
  - [ ] Run multi-step task
  - [ ] View in UI
  - [ ] Graph shows correct flow
  - Result: ________

---

## Phase 6: iOS Bridge (macOS Only) 📱

- [ ] **IOS-01**: Setup Script
  ```bash
  cd ios && sh setup_ios_bridge.sh
  ```
  - [ ] Script completes
  - [ ] No errors
  - Result: ________

- [ ] **IOS-02**: Simulator Test
  - [ ] Start iOS simulator
  - [ ] Run test against simulator
  - [ ] Communication successful
  - Result: ________

- [ ] **IOS-03**: Command Validation
  - [ ] Monitor agent logs
  - [ ] No "simctl tap" used
  - [ ] No "simctl swipe" used
  - [ ] Uses XCTest/AXP/AppleScript instead
  - Result: ________

- [ ] **IOS-04**: Advanced Harness
  - [ ] Run complex UI test
  - [ ] Drag-drop works
  - [ ] Multi-step forms work
  - Result: ________

---

## Phase 7: Infrastructure 🔧

### MCP Protocol
- [ ] **MCP-01**: Server Function
  ```bash
  target/release/arkavo serve
  ```
  - [ ] Server starts
  - [ ] Accepts connections
  - [ ] Tool calls work
  - Result: ________

- [ ] **MCP-02**: Tool Discovery
  - [ ] Query available tools
  - [ ] All tools listed
  - [ ] Schemas correct
  - Result: ________

### Memory Management
- [ ] **MEM-01**: Event Retention (1hr test)
  - [ ] Set retention to 1 hour
  - [ ] Generate events
  - [ ] Wait 1 hour
  - [ ] Old events pruned
  - Result: ________

- [ ] **MEM-02**: Max Events
  - [ ] Set max to 100
  - [ ] Generate 150 events
  - [ ] Only 100 retained
  - Result: ________

### Security & Networking
- [ ] **SEC-01**: mTLS Auth
  - [ ] Configure certificates
  - [ ] Connect to secure endpoint
  - [ ] Authentication successful
  - Result: ________

- [ ] **WS-01**: WebSocket
  - [ ] Connect WebSocket client
  - [ ] Send/receive messages
  - [ ] Connection stable
  - Result: ________

- [ ] **BUDGET-01**: Cost Limits
  - [ ] Set $0.10 limit
  - [ ] Run tasks
  - [ ] Warning at 90%
  - [ ] Stops at 100%
  - Result: ________

---

## Phase 8: Platform & Performance 🚀

### Platform Tests
- [ ] **PLAT-01**: macOS Full Suite
  - [ ] Run all CLI tests
  - [ ] Run all Agent tests
  - [ ] No platform errors
  - Result: ________

- [ ] **PLAT-02**: Linux x64
  - [ ] Deploy to Linux x64
  - [ ] Run core tests
  - [ ] No compatibility issues
  - Result: ________

- [ ] **PLAT-03**: Linux aarch64
  - [ ] Deploy to ARM Linux
  - [ ] Run core tests
  - [ ] No compatibility issues
  - Result: ________

- [ ] **PLAT-04**: macOS Notarization
  - [ ] Run notarization script
  - [ ] Package created
  - [ ] Ready for submission
  - Result: ________

### Performance
- [ ] **PERF-02**: Response Time
  - [ ] Measure chat response time
  - [ ] First token ≤50ms
  - [ ] Full response reasonable
  - Result: ________

- [ ] **PERF-03**: Metal NPU (macOS)
  - [ ] Run local model
  - [ ] Monitor GPU usage
  - [ ] NPU utilized
  - [ ] Better than CPU-only
  - Result: ________

### Protocols
- [ ] **A2A-01**: A2A Protocol
  - [ ] Start 2 agents
  - [ ] Exchange messages
  - [ ] Encryption verified
  - [ ] Authentication works
  - Result: ________

---

## Final Verification

### Regression Tests
- [ ] **REG-01**: Run Full Regression Suite
  ```bash
  .github/workflows/regression.yaml
  ```
  - [ ] All tests pass
  - [ ] No new regressions
  - Result: ________

### Documentation
- [ ] **DOC-01**: README Accuracy
  - [ ] Follow setup instructions
  - [ ] All commands work
  - [ ] Instructions clear
  - Result: ________

### Provider Switching
- [ ] **LLM-05**: Multi-Provider
  - [ ] Start with OpenAI
  - [ ] Switch to local model
  - [ ] Switch to Ollama
  - [ ] No state conflicts
  - Result: ________

- [ ] **LLM-06**: Vision Models
  - [ ] Test with image input
  - [ ] Accurate analysis
  - [ ] Multiple formats supported
  - Result: ________

### Rate Limiting & Errors
- [ ] **OPENAI-06**: Model Switching
  - [ ] Switch between GPT models
  - [ ] Context preserved
  - Result: ________

- [ ] **OPENAI-07**: Rate Limits
  - [ ] Send concurrent requests
  - [ ] Graceful handling
  - [ ] Retry logic works
  - Result: ________

- [ ] **OPENAI-08**: Auth Errors
  - [ ] Use invalid API key
  - [ ] Clear error message
  - [ ] No crash
  - Result: ________

---

## Test Summary

### Statistics
- Total Tests: 79
- Completed: ___
- Passed: ___
- Failed: ___
- Skipped: ___

### Critical Issues
1. ________________________________
2. ________________________________
3. ________________________________

### Recommendations
1. ________________________________
2. ________________________________
3. ________________________________

### Sign-off
- [ ] All critical tests passed
- [ ] Known issues documented
- [ ] Ready for release

**Tester**: _________________
**Date**: ___________________
**Signature**: ______________