# Stub Implementations TODO

This document tracks all stub/mock implementations that need to be replaced with real functionality.

## Critical Priority (Block core functionality)

### 1. State Management Tools (`crates/arkavo-test/src/mcp/server.rs`)
- [ ] `QueryStateKit::execute()` - Returns hardcoded "mocked_state" (line 200)
- [ ] `MutateStateKit::execute()` - Always returns success without actual mutation (line 256)
- [ ] `SnapshotKit::execute()` - Needs verification of actual snapshot functionality

**Fix**: Implement shared state store in McpTestServer and pass to tools

### 2. Core CLI Commands
- [ ] `chat` command (`crates/arkavo-cli/src/commands/chat.rs`) - Just echo loop
- [ ] `apply` command (`crates/arkavo-cli/src/commands/apply.rs`) - Completely unimplemented
- [ ] `vault` command (`crates/arkavo-cli/src/commands/vault.rs`) - Completely unimplemented

## Medium Priority (Enhanced functionality)

### 3. iOS UI Interaction (`crates/arkavo-test/src/mcp/ios_tools.rs`)
- [ ] Tap coordinates are hardcoded (lines 65-83)
- [ ] Text input uses echo instead of real UI interaction (lines 144-147)
- [ ] UI query returns hardcoded elements (lines 304-362)
- [ ] Fallback device "MOCK-DEVICE-ID" (line 414)

### 4. AI Analysis (`crates/arkavo-test/src/ai/analysis_engine.rs`)
- [ ] All methods return mock data when no API key:
  - `mock_analysis()` (line 192)
  - `mock_properties()` (line 222)
  - `mock_test_cases()` (line 244)
  - `mock_bug_analysis()` (line 268)

### 5. Intelligent Test Runner (`crates/arkavo-test/src/execution/intelligent_runner.rs`)
- [ ] `find_code_files()` returns hardcoded list (line 186)
- [ ] `mock_code_for_file()` returns placeholder (line 207)
- [ ] `simulate_test_execution()` doesn't run real tests (line 233)

## Low Priority (Can work with current implementation)

### 6. Test Command Modes (`crates/arkavo-cli/src/commands/test.rs`)
- [ ] Intelligent test modes return simple output (lines 243-298)

## Implementation Plan

### Phase 1: State Management (Today)
1. Add shared state store to McpTestServer
2. Update QueryStateKit to use real state
3. Update MutateStateKit to modify state
4. Update SnapshotKit to save/restore state

### Phase 2: Core Commands (High Priority)
1. Implement basic chat command with streaming
2. Implement apply command to execute plans
3. Implement vault command basics

### Phase 3: iOS Tools (Medium Priority)
1. Use xcrun simctl for real interactions
2. Parse actual UI hierarchy
3. Implement proper screenshot capture

### Phase 4: AI Integration (When API keys available)
1. Connect to real AI services
2. Remove mock responses
3. Add proper error handling

## Notes
- Some stubs may be acceptable for initial release if they don't block core functionality
- Focus on Phase 1 & 2 for MVP
- iOS tools can remain partially stubbed if not targeting iOS initially
- AI mocks are acceptable fallbacks when no API key is provided