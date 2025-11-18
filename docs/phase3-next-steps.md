# Phase 3: Next Session - Implementation Status & Action Items

## Current Status: ✅ Infrastructure Complete, Testing Needed

All code changes are committed and ready. The pipeline needs actual execution to validate improvements.

## What Was Completed This Session

### 1. Fixed Gemini API Connectivity ✅ (CRITICAL)
**Problem**: 30-second timeout caused all requests to fail
**Solution**: Switched from REST API to SSE streaming API
**Commit**: `e99e7d95` - Switch Gemini provider to streaming API

**Changes**:
- `GeminiProvider::complete_with_options`: Now uses `stream_generate_content()` and collects chunks
- `GeminiProvider::complete_with_tools`: Streams with tools, collects chunks + function_calls
- `RestClient` timeout: Increased 30s → 120s (fallback for non-streaming)

**Impact**: All 3 test instances completed without HTTP errors (was 1/3 timing out)

### 2. Improved Patch Generation Prompts ✅
**Problem**: Typo in prompt (`@@@` instead of `@@`), weak guidance
**Solution**: Enhanced prompt with explicit requirements
**Commit**: `fbf9ea35` - Improve patch generation prompt instructions

**Changes** (`crates/arkavo-context/src/prompt_enricher.rs:223-261`):
- Fixed critical typo: `@@@` → `@@` in hunk header example
- Added **CRITICAL REQUIREMENTS** section
- Emphasized using EXACT file paths from context
- Required minimum 3 lines of context before/after changes
- Explicit unified diff format specification

**Impact**: Better guidance, but patches still corrupt (wrong root cause)

### 3. Added Model Routing for Code Generation ✅ (KEY FIX)
**Problem**: Router selected tiny 270M model for complex patch generation
**Solution**: Created CodeGeneration task category → routes to 4B model
**Commit**: `c1864f1d` - Add CodeGeneration task category and improve model routing

**Changes**:
- **New TaskCategory**: `CodeGeneration` with keywords: `code_generation`, `codegen`, `patch`, `diff`, `generate`
- **Model selection**: CodeGeneration → LocalGemma4B (4B params vs 270M)
- **Auto-discovery**: Checks `/Volumes/SSD/huggingface/hub/models--unsloth--gemma-3-4b-it-GGUF` and `~/.cache/huggingface`
- **Task hint update**: CodeSolver now uses `"code_generation: Generate a unified git diff patch..."`
- **Token estimates**: 800 input, 3000 output (was using General: 300/1000)

**Files Modified**:
- `crates/arkavo-router/src/classifier.rs` - Add CodeGeneration category
- `crates/arkavo-router/src/selector.rs` - Route to LocalGemma4B
- `crates/arkavo-router/src/lib.rs` - Auto-discover gemma-3-4b-it.gguf
- `crates/arkavo-router/src/prediction.rs` - Add time estimates
- `crates/arkavo-orchestrator/src/code_solver.rs` - Update task hint

**Expected Impact**: 15x larger model should generate complete patches instead of 40-60 token fragments

### 4. Security: Removed Committed API Key ✅
**Problem**: GitGuardian alert #22516418 - expired key in git history
**Solution**: Rewritten git history, force pushed
**Commit**: `e728f0f4` - Remove hardcoded API key from documentation

**Actions**:
- Removed key from `docs/phase3-next-session.md`
- Rewrote git history with `git filter-branch`
- Force pushed to `feature/arkavo-assisted-benchmarking`
- Verified remote branch is clean

## Test Results Summary

### Before Fixes (Initial Phase 3 Run)
```
Instance 1: 121s - TIMEOUT ❌ (HTTP error at 30s limit)
Instance 2:  85s - Patch corrupt ❌
Instance 3:  31s - Patch failed to apply ❌
Resolution: 0/3 (0%)
```

### After Streaming API Fix
```
Instance 1: 101s - Patch corrupt ❌ (no timeout!)
Instance 2:  43s - Patch corrupt ❌
Instance 3:  37s - Patch corrupt ❌
Resolution: 0/3 (0%)
```
**Progress**: ✅ Connectivity fixed, but patch quality still broken

### After Prompt Improvements
```
Instance 1:  82s - Patch corrupt ❌
Instance 2:  59s - Patch corrupt ❌
Instance 3:  55s - Patch corrupt ❌
Resolution: 0/3 (0%)
```
**Progress**: ⚠️ Still using wrong model (270M generating 40-60 tokens)

### Root Cause Identified
**All instances using gemma-3-270m model** → generating 40-60 tokens when 500-1000+ needed for patches

**Log evidence**:
```
Generated 42 tokens in 5.67s (7.4 tok/s)  ← Instance 1
Generated 60 tokens in 7.05s (8.5 tok/s)  ← Instance 2
Generated 52 tokens in 5.31s (9.8 tok/s)  ← Instance 3
```

## Next Session: Action Items

### Priority 1: Verify Model Loading (CRITICAL)
**Goal**: Confirm gemma-3-4b-it loads from `/Volumes/SSD`

**Steps**:
1. Run with debug logging:
```bash
GEMINI_API_KEY=<your-key> \
NUM_INSTANCES=1 \
RUST_LOG=arkavo_router=debug,arkavo_llm=debug \
cargo run --example swe-bench-arkavo-phase3 2>&1 | grep -E "(Loading model|Model path|gemma)"
```

2. Look for:
   - `Loading model from: /Volumes/SSD/huggingface/hub/...`
   - `Model: gemma-3-4b-it` (not gemma-3-270m)
   - Token generation counts >100 tokens

3. If 270M still loading:
   - Check if 4B model file exists: `ls /Volumes/SSD/huggingface/hub/models--unsloth--gemma-3-4b-it-GGUF/snapshots/*/gemma-3-4b-it-Q4_0.gguf`
   - Set explicit path: `export ARKAVO_GEMMA_4B_PATH=/path/to/gemma-3-4b-it-Q4_0.gguf`
   - Check router decision logic with added debug prints

### Priority 2: Run Phase 3 with 4B Model
**Goal**: Validate that 4B model generates valid patches

**Command**:
```bash
GEMINI_API_KEY=<your-key> \
NUM_INSTANCES=3 \
RUST_LOG=arkavo_bench=info,arkavo_orchestrator=info \
timeout 900 cargo run --example swe-bench-arkavo-phase3
```

**Success Criteria**:
- Patches are >100 lines (not 40-60 tokens)
- At least 1/3 instances resolves successfully
- Patches apply cleanly (even if tests fail)

**Failure Indicators**:
- Still seeing 40-60 token outputs → model not loading
- "Corrupt patch" errors → may need max_tokens increase
- Line number mismatches → context extraction issue

### Priority 3: Adjust Configuration Based on Results

**If patches still short (<100 tokens)**:
- Increase llama.cpp max_tokens limit (currently defaults to model max)
- Check model's n_ctx setting in LlamaCppProvider

**If patches apply but tests fail**:
- ✅ This is EXPECTED - proceed to full 10 instance run
- Quality gate and test execution working

**If 1-2/3 resolve successfully**:
- ✅ HUGE SUCCESS - validates Arkavo approach!
- Run full 10 instance evaluation
- Compare against Phase 1 baseline (0% resolution)

### Priority 4: Full Phase 3 Evaluation (If 4B works)
**Goal**: Generate statistically meaningful results

**Command**:
```bash
GEMINI_API_KEY=<your-key> \
NUM_INSTANCES=10 \
RUST_LOG=arkavo_bench=info \
timeout 1800 cargo run --example swe-bench-arkavo-phase3 2>&1 | tee /tmp/phase3-full-run.log
```

**Analysis**:
```bash
# Extract metrics
cat /tmp/arkavo-phase3-*/phase3-metrics.json | jq '.summary'

# View failures
cat /tmp/arkavo-phase3-*/phase3-metrics.json | jq '.instances[] | select(.resolved == false)'

# Compare vs Phase 1 baseline
# Phase 1: 0% resolution (raw Gemini Flash)
# Phase 3 target: 70%+ resolution (Arkavo-assisted)
```

**Success Criteria**:
- ✅ 70%+ resolution rate (7+ of 10 resolved)
- ✅ 85%+ quality gate pass rate
- ✅ Average time <90s per instance
- ✅ Cost <$0.01 per instance

### Priority 5: Document Results
**Create**: `docs/phase3-results-YYYY-MM-DD.md`

**Include**:
- Resolution rate vs Phase 1 baseline
- Quality gate effectiveness
- Per-instance breakdown
- Error analysis for failures
- Cost and performance metrics
- Recommendations for Phase 4

**Update**:
- Issue #350 - GitHub Issue Resolution Integration
- PR #351 - Arkavo-Assisted Benchmarking Implementation

## Known Limitations

### Current Implementation
- **No iterative refinement**: Single-shot solution generation
- **No test feedback loop**: Doesn't retry based on test failures
- **Static context**: Doesn't adapt search based on initial results
- **Sequential execution**: One instance at a time (can be parallelized)
- **Fixed model**: Uses only LocalGemma4B, no dynamic escalation

### Model Constraints
- **4B model context**: Limited to 2048-4096 tokens (may truncate large files)
- **Generation speed**: ~10 tok/s (slower than cloud APIs)
- **Quality ceiling**: 4B models have limits on complex reasoning

### Test Framework Support
- ✅ pytest (Python)
- ✅ cargo test (Rust)
- ✅ jest/npm test (JavaScript)
- ❌ Other frameworks default to generic PASS/FAIL detection

## Optimization Opportunities (Post-Phase 3)

### If Resolution Rate <70%
1. **Improve context relevance**: Better keyword extraction from problem statement
2. **Increase context size**: Raise max_context_tokens from 8000 to 12000
3. **Add iterative refinement**: Retry with test failure feedback
4. **Better file selection**: Use AST-based relevance scoring

### If Quality Gate <85%
1. **Tune ResponseJudge**: Adjust prompts or use larger judge model
2. **Add validation**: Check solution before sending to judge
3. **Model selection**: Escalate to Gemini Pro for complex problems

### If Patches Still Malformed
1. **Increase max_tokens**: Configure llama.cpp to allow 2000+ output tokens
2. **Use Gemini for generation**: Route CodeGeneration to GeminiFlash instead of local
3. **Post-process patches**: Add validation layer to fix common formatting issues
4. **Switch to 7B+ model**: Use larger local model like CodeLlama 7B

## Technical Debt / Future Work

### Phase 4: Scale & Optimize
1. **Scale to 50+ instances** for statistical significance
2. **A/B test optimizations** (different models, context sizes)
3. **Iterative refinement** with test feedback loops
4. **Multi-model ensemble** (combine predictions)
5. **Semantic caching** of analyzed code contexts

### Phase 5: Production
1. **GitHub webhook integration** for real-time issue resolution
2. **Cost optimization** with budget constraints
3. **Human-in-the-loop** for review/approval
4. **Continuous benchmarking** on new SWE-bench releases

## Quick Reference Commands

```bash
# Check model location
ls -lh /Volumes/SSD/huggingface/hub/models--unsloth--gemma-3-4b-it-GGUF/snapshots/*/gemma-3-4b-it-Q4_0.gguf

# Set explicit model path (if auto-discovery fails)
export ARKAVO_GEMMA_4B_PATH=/Volumes/SSD/huggingface/hub/models--unsloth--gemma-3-4b-it-GGUF/snapshots/<hash>/gemma-3-4b-it-Q4_0.gguf

# Quick validation (1 instance)
GEMINI_API_KEY=<key> NUM_INSTANCES=1 cargo run --example swe-bench-arkavo-phase3

# Full Phase 3 run (10 instances)
GEMINI_API_KEY=<key> NUM_INSTANCES=10 timeout 1800 cargo run --example swe-bench-arkavo-phase3 2>&1 | tee /tmp/phase3-full.log

# View results
cat /tmp/arkavo-phase3-*/phase3-metrics.json | jq '.summary'

# Debug model loading
RUST_LOG=arkavo_router=debug,arkavo_llm=debug cargo run --example swe-bench-arkavo-phase3 2>&1 | grep -i "model\|gemma"
```

## Files & References

### Modified Files (This Session)
- `crates/arkavo-gemini/src/rest_client.rs` - Timeout increase
- `crates/arkavo-llm/src/gemini_adapter.rs` - Streaming API
- `crates/arkavo-context/src/prompt_enricher.rs` - Prompt improvements
- `crates/arkavo-router/src/classifier.rs` - CodeGeneration category
- `crates/arkavo-router/src/selector.rs` - Model routing
- `crates/arkavo-router/src/lib.rs` - Auto-discovery for 4B model
- `crates/arkavo-router/src/prediction.rs` - Time estimates
- `crates/arkavo-orchestrator/src/code_solver.rs` - Task hint

### Key Documentation
- `docs/phase3-next-session.md` - Previous execution guide (deprecated, use this file)
- `docs/arkavo-vs-raw-llm-comparison.md` - Expected improvements
- `crates/arkavo-bench/README.md` - API documentation

### Related Issues
- Issue #350: GitHub Issue Resolution Integration
- PR #351: Arkavo-Assisted Benchmarking Implementation
- GitGuardian Alert #22516418: Resolved (API key removed)

---

**Status**: Ready for next session. All infrastructure is in place. The critical blocker (model routing) is fixed but needs validation.

**Expected Outcome**: With gemma-3-4b-it properly loading, we should see significant improvement in patch quality and resolution rate.
