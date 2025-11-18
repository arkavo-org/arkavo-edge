# Gemini 3 Pro Preview Test Plan for Arkavo Edge

## Overview

This test plan validates Google's Gemini 3 Pro Preview (`models/gemini-3-pro-preview`) integration with Arkavo Edge. The plan is designed to be executed by Claude Code autonomously.

## Test Categories

### 1. Model Availability & Configuration
**Priority:** Critical
**Duration:** ~2 minutes

- [ ] Verify model is listed in available models
- [ ] Confirm token limits (1M input, 65K output)
- [ ] Validate streaming support flag
- [ ] Check model capabilities JSON

**Command:**
```bash
GEMINI_API_KEY=$GEMINI_API_KEY cargo run -p arkavo-gemini --example list_models 2>&1 | grep -A10 "gemini-3-pro-preview"
```

**Success Criteria:**
- Model appears in list
- Token limits match specification
- Streaming enabled

### 2. Basic Text Generation
**Priority:** Critical
**Duration:** ~5 minutes

**Test Cases:**
1. Simple prompt without tools
2. Multi-turn conversation
3. Long context (>10K tokens)
4. Code generation request
5. Markdown formatting

**Command:**
```bash
GEMINI_API_KEY=$GEMINI_API_KEY GEMINI_MODEL=models/gemini-3-pro-preview \
  cargo run -p arkavo -- chat --prompt "Write a Rust function to calculate fibonacci numbers"
```

**Success Criteria:**
- Response received within 5 seconds (TTFT)
- Valid Rust code generated
- Proper markdown formatting
- No JSON parsing errors

### 3. Function Calling & Tool Execution
**Priority:** Critical
**Duration:** ~10 minutes

**Test Cases:**
1. Single tool call (filesystem_tools)
2. Multiple tool calls in sequence
3. Tool call with complex parameters
4. Tool call error handling
5. Tool call retry logic

**Commands:**
```bash
# Test 1: Single tool call
GEMINI_API_KEY=$GEMINI_API_KEY GEMINI_MODEL=models/gemini-3-pro-preview \
  cargo run -p arkavo -- task "List files in crates/arkavo-mcp-tools/src"

# Test 2: Multiple tool calls
GEMINI_API_KEY=$GEMINI_API_KEY GEMINI_MODEL=models/gemini-3-pro-preview \
  cargo run -p arkavo -- task "List files in src/ and then read the first 10 lines of lib.rs"

# Test 3: Git operations
GEMINI_API_KEY=$GEMINI_API_KEY GEMINI_MODEL=models/gemini-3-pro-preview \
  cargo run -p arkavo -- task "Show git status and recent commits"
```

**Success Criteria:**
- All tool calls execute successfully
- Tool results are incorporated into response
- No "Tool not found" errors
- Proper error handling for invalid tool calls

### 4. Streaming Performance
**Priority:** High
**Duration:** ~10 minutes

**Test Cases:**
1. Measure TTFT (Time To First Token)
2. Measure throughput (tokens/second)
3. Validate streaming chunk delivery
4. Test stream interruption handling
5. Validate SSE event parsing

**Command:**
```bash
GEMINI_API_KEY=$GEMINI_API_KEY GEMINI_MODEL=models/gemini-3-pro-preview \
  cargo run -p arkavo-gemini --example streaming_tool_test
```

**Success Criteria:**
- TTFT < 5 seconds
- Throughput > 80 tokens/second
- No dropped chunks
- Graceful stream error handling

### 5. Complex Reasoning Tasks
**Priority:** High
**Duration:** ~20 minutes

**Test Cases:**
1. Multi-step code analysis
2. Bug diagnosis and fix generation
3. Architectural design questions
4. Code refactoring suggestions
5. Test generation from code

**Commands:**
```bash
# Test 1: Code analysis
GEMINI_API_KEY=$GEMINI_API_KEY GEMINI_MODEL=models/gemini-3-pro-preview \
  cargo run -p arkavo -- task "Analyze crates/arkavo-mcp-tools/src/filesystem.rs for potential improvements"

# Test 2: Bug diagnosis
GEMINI_API_KEY=$GEMINI_API_KEY GEMINI_MODEL=models/gemini-3-pro-preview \
  cargo run -p arkavo -- task "Review recent commits and identify any potential bugs or issues"

# Test 3: Test generation
GEMINI_API_KEY=$GEMINI_API_KEY GEMINI_MODEL=models/gemini-3-pro-preview \
  cargo run -p arkavo -- task "Generate unit tests for the ToolRegistry alias lookup functionality"
```

**Success Criteria:**
- Thoughtful, detailed analysis
- Actionable recommendations
- Code examples are valid
- Proper understanding of context

### 6. SWE-bench Style Tasks
**Priority:** High
**Duration:** ~30 minutes

**Test Cases:**
1. Issue understanding and diagnosis
2. Code changes to fix issues
3. Test generation for fixes
4. Documentation updates
5. End-to-end problem solving

**Command:**
```bash
GEMINI_API_KEY=$GEMINI_API_KEY NUM_INSTANCES=3 \
  cargo run -p arkavo-bench --example gemini-3-quick-test
```

**Success Criteria:**
- Successfully completes coding challenges
- Generates valid unified diffs
- Tests pass after applying fixes
- Documentation is clear and accurate

### 7. Long Context Handling
**Priority:** Medium
**Duration:** ~15 minutes

**Test Cases:**
1. Process large file (>50KB)
2. Analyze multiple files (>100KB total)
3. Summarize long conversation history
4. Extract information from large codebase
5. Handle token limit gracefully

**Commands:**
```bash
# Test 1: Large file analysis
GEMINI_API_KEY=$GEMINI_API_KEY GEMINI_MODEL=models/gemini-3-pro-preview \
  cargo run -p arkavo -- task "Analyze all files in crates/arkavo-mcp-tools/src and provide a summary"

# Test 2: Multi-file reasoning
GEMINI_API_KEY=$GEMINI_API_KEY GEMINI_MODEL=models/gemini-3-pro-preview \
  cargo run -p arkavo -- task "Compare the implementation of filesystem tools in arkavo-mcp-tools vs arkavo-mcp-macos"
```

**Success Criteria:**
- Successfully processes large contexts
- Maintains accuracy with long input
- Graceful degradation near token limits
- Proper context window management

### 8. Error Handling & Edge Cases
**Priority:** Medium
**Duration:** ~10 minutes

**Test Cases:**
1. Invalid API key handling
2. Network timeout handling
3. Malformed tool call recovery
4. Rate limit handling
5. Model overload (503 errors)

**Commands:**
```bash
# Test 1: Invalid API key
GEMINI_API_KEY=invalid cargo run -p arkavo -- chat --prompt "test"

# Test 2: Timeout simulation (short timeout)
timeout 5 GEMINI_API_KEY=$GEMINI_API_KEY cargo run -p arkavo -- task "Complex analysis task"
```

**Success Criteria:**
- Clear error messages
- No panics or crashes
- Proper error propagation
- Retry logic works correctly

### 9. Comparison with Gemini 2.5 Pro
**Priority:** Low
**Duration:** ~20 minutes

**Test Cases:**
1. Same prompt to both models
2. Compare response quality
3. Compare latency
4. Compare token usage
5. Compare cost (when pricing available)

**Commands:**
```bash
# Gemini 3 Pro Preview
GEMINI_API_KEY=$GEMINI_API_KEY GEMINI_MODEL=models/gemini-3-pro-preview \
  cargo run -p arkavo -- chat --prompt "Explain the tool alias system" > /tmp/gemini3.txt

# Gemini 2.5 Pro
GEMINI_API_KEY=$GEMINI_API_KEY GEMINI_MODEL=gemini-2.5-pro \
  cargo run -p arkavo -- chat --prompt "Explain the tool alias system" > /tmp/gemini2.txt

# Compare
diff /tmp/gemini3.txt /tmp/gemini2.txt
```

**Success Criteria:**
- Both models produce valid responses
- Quality is comparable or better in 3 Pro
- Latency tradeoffs are acceptable
- Cost is justified by quality (when known)

### 10. Integration Tests
**Priority:** High
**Duration:** ~15 minutes

**Test Cases:**
1. Router selection logic
2. Cost tracking
3. Budget limits
4. Health monitoring
5. Metrics collection

**Commands:**
```bash
# Test router with Gemini 3 Pro
RUST_LOG=arkavo_router=debug cargo test -p arkavo-router

# Test cost tracking
cargo test -p arkavo-budget cost

# Test health checks
cargo test -p arkavo-protocol health
```

**Success Criteria:**
- Router selects appropriate model
- Costs are tracked accurately
- Budget limits are enforced
- Health checks pass
- Metrics are collected

## Automated Test Suite

Create a single test script that runs all critical tests:

**File:** `crates/arkavo-bench/examples/gemini-3-comprehensive-test.rs`

### Test Execution Order

1. **Smoke Tests** (2 min)
   - Model availability
   - Basic text generation
   - Single tool call

2. **Core Functionality** (15 min)
   - Function calling suite
   - Streaming performance
   - Error handling

3. **Advanced Features** (30 min)
   - Complex reasoning
   - SWE-bench tasks
   - Long context handling

4. **Integration** (15 min)
   - Router integration
   - Cost tracking
   - Health monitoring

5. **Comparison** (20 min)
   - Side-by-side with 2.5 Pro
   - Quality assessment
   - Performance metrics

**Total Duration:** ~80 minutes

## Test Results Format

Each test should output:
```json
{
  "test_name": "basic_text_generation",
  "category": "critical",
  "status": "passed",
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

## Success Criteria Summary

### Critical (Must Pass)
- ✅ Model availability confirmed
- ✅ Basic text generation works
- ✅ Tool calling functional
- ✅ Streaming works correctly
- ✅ No crashes or panics

### High Priority (Should Pass)
- ✅ Complex reasoning tasks complete successfully
- ✅ SWE-bench style tasks work
- ✅ Long context handled properly
- ✅ Integration tests pass

### Medium Priority (Nice to Have)
- ✅ Error handling graceful
- ✅ Comparison favorable vs 2.5 Pro
- ✅ Performance metrics acceptable

## Deliverables

1. **Test Results Report**
   - Summary of all test outcomes
   - Performance metrics table
   - Failure analysis (if any)
   - Recommendations

2. **Test Artifacts**
   - Log files for each test
   - Performance graphs
   - Example outputs
   - Error traces (if any)

3. **Production Readiness Assessment**
   - Go/No-Go recommendation
   - Known limitations
   - Deployment guidance
   - Monitoring recommendations

### 11. Automated Integration Tests (Phase 7)
**Priority:** Critical
**Duration:** ~5 minutes

**Test Cases:**
1. Parallel tool execution
2. Multi-turn conversation consistency
3. JSON mode reliability
4. Tool orchestration complexity (3+ tools)
5. Error recovery from malformed tool responses

**Command:**
```bash
cargo test -p arkavo-gemini --test gemini_3_integration_test
```

**Success Criteria:**
- All integration tests pass
- Parallel tool calls execute in single turn
- JSON schema compliance 100%
- Complex tool orchestration succeeds

### 12. Multimodal Capabilities (Phase 8)
**Priority:** High
**Duration:** ~5 minutes

**Test Cases:**
1. Image input (base64 encoded PNG)
2. Vision-language integration
3. Combined image and text input
4. Multimodal error handling

**Command:**
```bash
cargo test -p arkavo-gemini --test multimodal_test
```

**Success Criteria:**
- Model acknowledges visual content
- No "text only model" errors
- Image+text queries work correctly
- Graceful error handling for invalid images

### 13. Safety & Edge Cases (Phase 9)
**Priority:** High
**Duration:** ~10 minutes

**Test Cases:**
1. Safety filter handling (SAFETY finish_reason)
2. Rate limit backoff (429 responses)
3. Context window overflow
4. Token limit boundary behavior
5. Invalid API key handling
6. Network timeout handling
7. Empty/very long prompts
8. Rapid sequential requests

**Command:**
```bash
cargo test -p arkavo-gemini --test safety_edge_cases_test
```

**Success Criteria:**
- Graceful error messages for all edge cases
- No panics or crashes
- Rate limiting handled with backoff
- Large contexts processed correctly

### 14. Quality Validation (Phase 10)
**Priority:** High
**Duration:** ~15 minutes

**Test Cases:**
1. Code compilation check (generated code must compile)
2. Needle in haystack (UUID retrieval from 100K tokens)
3. Factual accuracy on known questions
4. JSON schema compliance scoring
5. Code explanation quality assessment

**Command:**
```bash
cargo run -p arkavo-bench --example gemini-3-quality-test
```

**Success Criteria:**
- Generated code compiles successfully
- Context retrieval accurate at scale (100K+ tokens)
- ≥67% factual accuracy
- ≥75% schema compliance
- Quality score ≥0.6 (good) or ≥0.8 (excellent)

## Automated Test Suite

### Comprehensive Test Runner

**File:** `crates/arkavo-bench/examples/gemini-3-comprehensive-test.rs`

Runs all phases 1-10 automatically:

```bash
GEMINI_API_KEY=$GEMINI_API_KEY cargo run -p arkavo-bench --example gemini-3-comprehensive-test
```

### Test Execution Order

1. **Smoke Tests** (5 min) - Phases 1-3
   - Model availability
   - Basic text generation
   - Single tool call

2. **Core Functionality** (20 min) - Phases 4-6
   - Streaming performance
   - Multiple tool calls
   - Error handling
   - SWE-bench tasks

3. **Production-Grade Tests** (30 min) - Phases 7-10
   - Automated integration tests
   - Multimodal capabilities
   - Safety & edge cases
   - Quality validation

**Total Duration:** ~55 minutes (enhanced from original 80 minutes)

## Test Implementation

### Automated Tests (cargo test)
Located in `crates/arkavo-gemini/tests/`:
- `gemini_3_integration_test.rs` - Programmatic assertions
- `multimodal_test.rs` - Vision capabilities
- `safety_edge_cases_test.rs` - Production robustness

### Benchmark Tests (examples)
Located in `crates/arkavo-bench/examples/`:
- `gemini-3-comprehensive-test.rs` - Full automated suite
- `gemini-3-quality-test.rs` - Output correctness validation
- `gemini-3-quick-test.rs` - Lightweight validation

## Updated Success Criteria Summary

### Critical (Must Pass 100%)
- ✅ Model availability confirmed
- ✅ Basic text generation works
- ✅ Tool calling functional
- ✅ Streaming works correctly
- ✅ No crashes or panics
- ✅ Automated integration tests pass

### High Priority (≥90% pass rate)
- ✅ Complex reasoning tasks complete
- ✅ SWE-bench style tasks work
- ✅ Multimodal capabilities validated
- ✅ Safety/edge cases handled gracefully
- ✅ Quality validation (code correctness)
- ✅ ≥85% test coverage for arkavo-gemini

### Medium Priority (≥80% pass rate)
- ✅ Error handling graceful
- ✅ Performance metrics acceptable
- ✅ Long context handled properly
- ✅ Integration tests pass

## Next Steps After Testing

1. Document any issues found
2. Create GitHub issues for bugs
3. Update documentation with findings
4. Update PR with test results
5. Recommend production deployment strategy
6. Verify ≥85% test coverage target met
