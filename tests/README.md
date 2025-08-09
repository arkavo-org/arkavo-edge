# Arkavo Edge Test Suite

This directory contains all test-related files for the Arkavo Edge project.

## Directory Structure

```
tests/
├── README.md                    # This file
├── docs/                        # Test documentation
│   ├── test-plan.md            # Comprehensive test plan (91 tests)
│   ├── manual-test-checklist.md # Manual test execution checklist
│   ├── manual-test-execution-plan.md # Phased execution plan
│   ├── test-results-*.md       # Test execution reports
│   └── IOS_TESTING_ENHANCED.md # iOS-specific testing guide
├── scripts/                     # Test automation scripts
│   ├── test-runner.sh          # Main automated test runner
│   ├── demo_agent_interaction.py # Agent interaction demo
│   └── launch_multi_agent_system.sh # Multi-agent test launcher
├── test-results/               # Test execution results
│   ├── results-*.json         # JSON test results
│   └── summary-*.md          # Human-readable summaries
├── integration/               # Integration test files
├── default_agent_run.rs      # Rust integration test
└── multi_agent_collaboration_test.rs # Multi-agent Rust test
```

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

### Terminal Relaunch Bug
**Critical**: The Arkavo binary automatically relaunches in Terminal.app when not in a TTY context, blocking automated testing of most CLI commands.

**Location**: `crates/arkavo/src/main.rs:40-77`

**Workaround**: None currently available. Fix requires adding environment variable check:
```rust
if std::env::var("ARKAVO_NO_TERMINAL_RELAUNCH").is_ok() {
    return; // Skip terminal relaunch
}
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