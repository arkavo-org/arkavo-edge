# OpenAI Integration Test Suite

This directory contains comprehensive tests for OpenAI integration with Arkavo Edge, including support for the newly released GPT-5 model.

## Prerequisites

### 1. OpenAI API Key
You need an OpenAI API key to run these tests. Get one from [OpenAI Platform](https://platform.openai.com/api-keys).

### 2. Environment Setup
Create a `.test.env` file in the project root with your API key:

```bash
# Copy the template
cp .test.env.example .test.env

# Edit and add your API key
OPENAI_API_KEY=your-api-key-here
```

### 3. Build the Project
```bash
cargo build --release
```

## Test Structure

```
tests/openai_integration/
├── mod.rs                      # Common test utilities
├── test_openai_basic.rs        # Basic connectivity and auth tests
├── test_openai_models.rs       # Model-specific tests (GPT-3.5, GPT-4, GPT-4o)
├── test_openai_streaming.rs    # Streaming response tests
├── test_openai_vision.rs       # Vision capabilities tests (GPT-4o)
├── test_openai_budget.rs       # Cost tracking and budget tests
├── test_openai_e2e.sh         # End-to-end CLI tests
├── config/
│   └── test_models.yaml       # Test configuration and model specs
└── README.md                  # This file
```

## Running Tests

### Run All Tests
```bash
# Run all OpenAI integration tests (requires API key)
cargo test --test '*openai*' -- --ignored --nocapture

# Run with verbose output
RUST_LOG=debug cargo test --test '*openai*' -- --ignored --nocapture
```

### Run Individual Test Files
```bash
# Basic connectivity tests
cargo test --test test_openai_basic -- --ignored --nocapture

# Model-specific tests
cargo test --test test_openai_models -- --ignored --nocapture

# Streaming tests
cargo test --test test_openai_streaming -- --ignored --nocapture

# Vision tests (requires GPT-4o access)
cargo test --test test_openai_vision -- --ignored --nocapture

# Budget tracking tests
cargo test --test test_openai_budget -- --ignored --nocapture
```

### Run Specific Tests
```bash
# Run a single test
cargo test test_openai_basic_connectivity -- --ignored --nocapture

# Run tests matching a pattern
cargo test gpt_4o -- --ignored --nocapture
```

### Run End-to-End CLI Tests
```bash
# Make sure the script is executable
chmod +x tests/openai_integration/test_openai_e2e.sh

# Run the E2E test suite
./tests/openai_integration/test_openai_e2e.sh
```

## Test Categories

### 1. Basic Connectivity (`test_openai_basic.rs`)
- API key authentication
- Connection establishment
- Error handling for invalid credentials
- Rate limiting behavior
- Provider factory validation

### 2. Model-Specific Tests (`test_openai_models.rs`)
- **GPT-5**: Latest generation model with unique characteristics
  - Temperature must be default (1.0) - custom values rejected
  - Requires organization verification for streaming
  - Response time: 3-6 seconds (non-streaming)
  - More concise responses than previous models
- Performance comparison between model variants
- JSON mode responses
- Model fallback handling

### 3. Streaming Tests (`test_openai_streaming.rs`)
- Basic streaming functionality
- Performance metrics (time to first token)
- Stream interruption handling
- Concurrent streaming requests
- Error handling in streams

### 4. Vision Tests (`test_openai_vision.rs`)
- Image analysis with GPT-5 (vision capabilities)
- Multiple image inputs
- Image + text prompts
- Vision streaming responses (requires org verification)
- Non-vision model behavior with images

### 5. Budget Tracking (`test_openai_budget.rs`)
- Token cost calculation
- Budget limit enforcement
- Warning thresholds
- Cost comparison between models
- Streaming cost tracking

### 6. End-to-End Tests (`test_openai_e2e.sh`)
- Full CLI integration
- Agent mode with OpenAI
- Model switching
- Multi-turn conversations
- Error recovery

## GPT-5 Specific Notes

GPT-5 was released recently and has unique characteristics:

### Requirements
- **Organization Verification**: Required for streaming endpoints
- **Temperature**: Only accepts default (1.0), custom values are rejected
- **API Access**: Standard OpenAI API key with GPT-5 access

### Performance Characteristics
- **Response Time**: 3-6 seconds for non-streaming
- **Streaming**: ~5 seconds to first token (when org verified)
- **Response Style**: More concise than GPT-4 models
- **Context**: Handles long contexts but with different behavior

### Known Limitations
- No custom temperature support
- Streaming requires verified organization
- Different response patterns may affect some tests

## Configuration

The `config/test_models.yaml` file contains:
- Model specifications (context windows, pricing)
- Test scenarios and prompts
- Retry and timeout settings
- Rate limiting configuration
- Budget limits for testing

## Cost Considerations

⚠️ **These tests use real OpenAI API calls and will incur costs!**

Estimated costs per full test run:
- Basic tests: ~$0.01
- Model tests: ~$0.10
- Streaming tests: ~$0.05
- Vision tests: ~$0.20
- Budget tests: ~$0.05
- **Total: ~$0.41 per full suite run**

To minimize costs:
1. Run individual test files instead of the full suite
2. Use GPT-3.5-turbo for most tests
3. Set budget limits in `.test.env`:
   ```
   TEST_BUDGET_LIMIT_USD=1.00
   ```

## Continuous Integration

For CI/CD pipelines:

1. **Store API key as a secret**:
   ```yaml
   env:
     OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
   ```

2. **Run a subset of tests**:
   ```bash
   # Only run basic connectivity in CI
   cargo test test_openai_basic_connectivity -- --ignored
   ```

3. **Use mock responses for most CI tests**:
   ```bash
   # Run tests without --ignored flag to use mocks
   cargo test --test '*openai*'
   ```

## Troubleshooting

### Common Issues

1. **Authentication Errors**
   - Verify your API key is correct in `.test.env`
   - Check if the key has the required permissions
   - Ensure the key hasn't expired

2. **Rate Limiting**
   - Tests include retry logic with exponential backoff
   - Reduce concurrent test execution if hitting limits
   - Consider using different API keys for parallel testing

3. **Model Access**
   - GPT-4 models require specific API access
   - GPT-4o vision features need additional permissions
   - Check your OpenAI account's model access

4. **Timeout Issues**
   - Increase timeout values in test code
   - Check network connectivity
   - Verify OpenAI service status

### Debug Mode

Enable detailed logging:
```bash
RUST_LOG=arkavo_dataflow=debug,arkavo_llm=debug cargo test -- --ignored --nocapture
```

## Adding New Tests

1. Create a new test file following the naming convention
2. Import the common module for shared utilities:
   ```rust
   #[path = "mod.rs"]
   mod common;
   use common::ensure_api_key;
   ```
3. Mark tests with `#[ignore]` to prevent running without API key
4. Update this README with the new test description

## Safety Guidelines

- Never commit API keys to version control
- Use `.gitignore` to exclude `.test.env`
- Implement budget limits to prevent excessive costs
- Monitor API usage through OpenAI dashboard
- Use the lowest-cost model that meets test requirements