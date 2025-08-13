# Arkavo Edge Test Suite

This directory contains all test-related files for the Arkavo Edge project.

## Running Tests

### Automated Tests
Run the automated test suite from the project root:
```bash
./tests/scripts/test-runner.sh
```

Or from within the tests directory:
```bash
cd tests
./scripts/test-runner.sh
```

### Multi-Agent System Tests
Run the comprehensive multi-agent system test suite:
```bash
./tests/scripts/test_multi_agent_system.sh
```

This test suite validates:
- **Configuration Updates**: Atomic updates, backup creation, version tracking
- **Telemetry UI**: Data collection, real-time updates, metrics aggregation
- **Ollama Remote Connectivity**: Remote server connectivity, model discovery, streaming
- **Agent Discovery**: mDNS service registration and discovery between agents
- **Multi-Agent Orchestration**: Startup and health checks for 10 agent types
- **Stress Testing**: Concurrent operations and high-volume telemetry

Expected results:
- **Agent Discovery**: Should complete in ≤5 seconds (typically 2-3s)
- **Remote Ollama**: May show slow performance warnings (>5s) depending on network
- **Configuration**: Some concurrent update tests may show race conditions
- **Telemetry UI**: Apple Events authorization may prevent UI launch on macOS

### Manual Tests
Follow the checklist in `docs/manual-test-checklist.md` for manual test execution.

### Integration Tests
Run Rust integration tests:
```bash
cargo test --test default_agent_run
cargo test --test multi_agent_collaboration_test
```

## Test Coverage

The test suite includes 91 total tests covering:
- Build & Setup (3 tests)
- CLI Core Functionality (5 tests)
- Agent Features (6 tests)
- Git Integration (3 tests)
- LLM Providers (15 tests)
- UI/TUI (7 tests)
- iOS Bridge (4 tests)
- Infrastructure & Security (7 tests)
- Platform & Performance (6 tests)
- Documentation (2 tests)
- Error Handling (2 tests)
- And more...

## Known Issues

### Terminal Relaunch Bug (FIXED)
**Status**: Fixed in commit [pending]

**Previous Issue**: The Arkavo binary automatically relaunched in Terminal.app when not in a TTY context, blocking automated testing.

**Solution**: Added `ARKAVO_NO_TERMINAL_RELAUNCH` environment variable to disable terminal relaunch for testing/automation.

**Usage**:
```bash
# For automated testing
export ARKAVO_NO_TERMINAL_RELAUNCH=1
./target/release/arkavo --help

# Or inline
ARKAVO_NO_TERMINAL_RELAUNCH=1 ./target/release/arkavo chat --prompt "test"
```

## Test Results

Test results are stored in `test-results/` with timestamps:
- JSON format: `results-YYYYMMDD-HHMMSS.json`
- Markdown summary: `summary-YYYYMMDD-HHMMSS.md`

## Contributing

When adding new tests:
1. Add test cases to `docs/test-plan.md`
2. Update automation in `scripts/test-runner.sh` if applicable
3. Document manual steps in `docs/manual-test-checklist.md`
4. Store results in `test-results/`