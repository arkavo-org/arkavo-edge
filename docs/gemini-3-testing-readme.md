# Gemini 3 Pro Preview Testing Guide

## Quick Start

### Prerequisites
```bash
export GEMINI_API_KEY=your_api_key_here
```

### Run Quick Smoke Tests (2-3 minutes)
```bash
./scripts/test-gemini-3.sh --quick
```

### Run Comprehensive Test Suite (10-15 minutes)
```bash
./scripts/test-gemini-3.sh
```

### Run Specific Tests Manually

#### Test Model Availability
```bash
cargo run -p arkavo-gemini --example list_models | grep gemini-3-pro-preview
```

#### Test Basic Generation
```bash
GEMINI_MODEL=models/gemini-3-pro-preview \
  cargo run -p arkavo -- chat --prompt "Write a Rust hello world function"
```

#### Test Tool Calling
```bash
GEMINI_MODEL=models/gemini-3-pro-preview \
  cargo run -p arkavo -- task "List files in crates/arkavo-mcp-tools/src"
```

#### Test Streaming Performance
```bash
GEMINI_MODEL=models/gemini-3-pro-preview \
  cargo run -p arkavo-gemini --example streaming_tool_test
```

#### Test SWE-bench Tasks
```bash
GEMINI_MODEL=models/gemini-3-pro-preview \
  cargo run -p arkavo-bench --example gemini-3-quick-test
```

## Test Categories

### Critical Tests (Must Pass)
1. **Model Availability** - Confirms model is accessible
2. **Basic Text Generation** - Tests simple prompt/response
3. **Single Tool Call** - Validates tool calling works

### High Priority Tests (Should Pass)
4. **Streaming Performance** - Measures TTFT and throughput
5. **Multiple Tool Calls** - Tests sequential tool execution
6. **SWE-bench Tasks** - Complex code generation

### Medium Priority Tests (Nice to Have)
7. **Error Handling** - Tests graceful failure modes

## Expected Performance

### Gemini 3 Pro Preview Benchmarks
- **TTFT (Time to First Token):** 2-3 seconds
- **Throughput:** 80-100 tokens/second
- **Tool Call Latency:** 2-3 seconds per call
- **Context Window:** 1M input, 65K output tokens

### Comparison with Gemini 2.5 Pro
| Metric | 2.5 Pro | 3 Pro Preview | Winner |
|--------|---------|---------------|--------|
| TTFT | 0.9s | 2.5s | 2.5 Pro (2.7x faster) |
| Throughput | ~110 tok/s | ~100 tok/s | 2.5 Pro |
| Reasoning Quality | Excellent | Excellent | Tie |
| Context Window | 2M tokens | 1M tokens | 2.5 Pro |

**Use Cases:**
- **Gemini 3 Pro Preview:** Complex reasoning, quality-critical work
- **Gemini 2.5 Pro:** Interactive sessions, latency-sensitive tasks

## Test Results Format

Results are saved to `gemini-3-test-results.json` with this structure:

```json
{
  "test_name": "basic_text_generation",
  "category": "Critical",
  "status": "Passed",
  "duration_ms": 3420,
  "metrics": {
    "ttft_ms": 2470,
    "tokens_generated": 156,
    "throughput_tps": 45.6
  },
  "errors": [],
  "notes": "Response quality excellent"
}
```

## Interpreting Results

### Production Ready Criteria
✅ **Ready for Production** if:
- All critical tests pass (100%)
- ≥80% of high-priority tests pass
- No panics or crashes detected
- Error handling is graceful

⚠️ **Not Ready** if:
- Any critical test fails
- <80% of high-priority tests pass
- Panics or crashes detected
- Poor error handling

### Common Issues

#### "Tool not found" errors
**Cause:** Filesystem tools not registered or alias not working
**Fix:** Ensure latest code with #355 fix is deployed

#### Authentication errors
**Cause:** Invalid or missing API key
**Fix:** Check `GEMINI_API_KEY` environment variable

#### Timeout errors
**Cause:** Model overloaded or network issues
**Fix:** Retry with longer timeout or try again later

#### Rate limit errors
**Cause:** Too many requests
**Fix:** Wait 60 seconds and retry with fewer concurrent tests

## Test Execution by Claude Code

This test suite is designed to be executed autonomously by Claude Code. To run:

1. **Set API Key:**
   ```bash
   export GEMINI_API_KEY=your_key
   ```

2. **Run Test Suite:**
   ```bash
   cargo run --example gemini-3-comprehensive-test
   ```

3. **Review Results:**
   ```bash
   cat gemini-3-test-results.json
   ```

4. **Check Summary:**
   The test runner will output a summary with:
   - Pass/fail counts
   - Performance metrics
   - Production readiness assessment

## Automated Testing in CI/CD

### GitHub Actions Integration

```yaml
name: Gemini 3 Pro Preview Tests

on:
  push:
    branches: [ main, feature/* ]
  schedule:
    - cron: '0 0 * * *'  # Daily

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - name: Run Gemini 3 Tests
        env:
          GEMINI_API_KEY: ${{ secrets.GEMINI_API_KEY }}
        run: |
          cargo run --example gemini-3-comprehensive-test
      - name: Upload Results
        uses: actions/upload-artifact@v3
        with:
          name: test-results
          path: gemini-3-test-results.json
```

## Troubleshooting

### Test hangs or times out
- Check internet connectivity
- Verify API key is valid
- Check Gemini API status page
- Increase timeout in test runner

### Tests fail intermittently
- This may indicate rate limiting
- Add delays between tests
- Reduce concurrent test execution

### Poor performance metrics
- Check system resources
- Verify network latency
- Try different time of day (API load varies)

## Contributing

When adding new tests:

1. Add test method to `TestRunner` struct
2. Call test method in `main()` function
3. Set appropriate test category (Critical/High/Medium)
4. Document expected behavior and success criteria
5. Update this README with new test details

## References

- [Gemini 3 Pro Preview Documentation](./gemini-3-pro-preview.md)
- [Gemini 2.5 vs 3 Comparison](./gemini-2.5-vs-3-pro-comparison.md)
- [Test Plan](./gemini-3-pro-preview-test-plan.md)
- [Issue #354](https://github.com/arkavo-org/arkavo-edge/issues/354)
- [Issue #355](https://github.com/arkavo-org/arkavo-edge/issues/355)
