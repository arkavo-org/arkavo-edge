# Test Plan for arkavo-llm Crate

## Overview
This test plan covers comprehensive testing of the arkavo-llm crate with Ollama API integration.

## Test Categories

### 1. Unit Tests

#### Message Types
- Create messages with different roles (system, user, assistant)
- Verify content preservation
- Test edge cases (empty strings, special characters)

#### Error Types
- Verify error conversion from underlying types
- Test error message formatting
- Ensure error context is preserved

#### Provider Trait
- Mock implementation testing
- Verify trait bounds and async behavior
- Test default implementations if any

### 2. Integration Tests

#### Ollama Client - Basic Functionality
- Test client creation with default settings
- Test client creation with custom base URL and model
- Test environment variable configuration
- Verify connection handling

#### Ollama Client - API Interactions
- Test successful completion requests
- Test streaming responses
- Handle API errors gracefully
- Test timeout scenarios
- Verify JSON parsing

### 3. End-to-End Tests

#### Real Ollama Server (when available)
- Send actual completion requests
- Verify streaming functionality
- Test different model configurations
- Measure response times

### 4. Error Scenarios

#### Network Errors
- Connection refused (Ollama not running)
- Timeout during request
- Invalid base URL
- Network interruption during streaming

#### API Errors
- Invalid model name
- Malformed request
- Rate limiting (if applicable)
- Server errors (5xx responses)

#### Data Errors
- Invalid JSON responses
- Incomplete streaming data
- Character encoding issues

### 5. Configuration Tests

#### Environment Variables
- LLM_PROVIDER selection
- OLLAMA_BASE_URL configuration
- OLLAMA_MODEL selection
- Missing environment variables
- Invalid environment values

### 6. Performance Tests

#### Streaming Performance
- Measure latency for first token
- Throughput for streaming responses
- Memory usage during long sessions
- Concurrent request handling

### 7. Security Tests

#### API Key Management
- Ensure no sensitive data in logs
- Verify secure storage practices
- Test credential rotation scenarios

## Test Implementation Plan

### Phase 1: Core Functionality (Priority: High)
1. Expand unit tests for all types
2. Add mock-based provider tests
3. Create integration test suite with mock server

### Phase 2: Robustness (Priority: Medium)
1. Implement comprehensive error scenario tests
2. Add timeout and retry logic tests
3. Test edge cases and boundary conditions

### Phase 3: Real Integration (Priority: High)
1. Create Docker-based test environment with Ollama
2. Implement real server integration tests
3. Add performance benchmarks

### Phase 4: Extended Coverage (Priority: Low)
1. Stress testing with concurrent requests
2. Long-running session tests
3. Memory leak detection

## Test Execution Strategy

### Local Development
```bash
# Run unit tests only
cargo test --lib

# Run all tests (requires Ollama running)
OLLAMA_BASE_URL=http://localhost:11434 cargo test

# Run with verbose output
cargo test -- --nocapture
```

### CI/CD Pipeline
- Unit tests run on every commit
- Integration tests with mock server on PRs
- Full integration tests with Ollama in Docker on main branch
- Performance benchmarks tracked over time

## Success Criteria
- 100% coverage of public API surface
- All error paths tested
- Performance within acceptable bounds
- No memory leaks or panics
- Clear documentation of test requirements