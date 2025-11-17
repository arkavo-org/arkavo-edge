# Phase 3: Execution Ready - Next Session Guide

## Current Status: ✅ READY FOR EXECUTION

All infrastructure is complete, tested, and committed. The pipeline is ready for actual SWE-bench evaluation.

## What Was Completed

### Infrastructure Built (Commit: 402fd3f9)
- ✅ Test runner: `crates/arkavo-bench/examples/swe-bench-arkavo-phase3.rs` (218 lines)
- ✅ Execution log: `docs/phase3-execution-log.md`
- ✅ 4 critical bugs fixed
- ✅ All code passes clippy, formatted, compiles successfully

### Critical Fixes Applied

#### 1. Repository Cloning ✅
**Problem**: Workspace was empty, CodeSolver had no files to analyze
**Fixed**: Added `clone_repository()` to clone GitHub repos and checkout base commit

#### 2. Recursive File Search ✅
**Problem**: Only searched root directory, missed files in subdirectories
**Fixed**: Implemented recursive traversal with depth limit, skips build artifacts

#### 3. Message Passing to Router ✅ **MOST CRITICAL**
**Problem**: "Provider error: No messages provided"
**Fixed**: Created proper `Message::user(enriched_prompt)` instead of empty vec

#### 4. Dependencies ✅
**Fixed**: Added uuid, chrono, tracing-subscriber to Cargo.toml

## How to Execute Phase 3

### Quick Start (Recommended)

Run with 1-3 instances first to validate everything works:

```bash
GEMINI_API_KEY=<your-key> \
NUM_INSTANCES=3 \
RUST_LOG=arkavo_bench=info,arkavo_orchestrator=info \
cargo run --example swe-bench-arkavo-phase3
```

### Full Phase 3 Run (10 instances)

```bash
GEMINI_API_KEY=<your-key> \
NUM_INSTANCES=10 \
RUST_LOG=arkavo_bench=info \
timeout 1800 cargo run --example swe-bench-arkavo-phase3 2>&1 | tee /tmp/phase3-full-run.log
```

### Environment Variables

- `GEMINI_API_KEY` - **Required**: Your Gemini API key (provided by user)
- `NUM_INSTANCES` - Optional: Number of instances to run (default: 10)
- `RUST_LOG` - Optional: Logging level (recommended: `arkavo_bench=info`)

### Expected Behavior

The runner will:
1. Load N SWE-bench Lite instances from HuggingFace
2. Initialize Router with quality gate (Gemini + ResponseJudge)
3. For each instance:
   - Create isolated workspace
   - Clone repository and checkout base commit
   - Run CodeSolver with recursive file search
   - Build enriched context (max 8000 tokens)
   - Generate solution via Router with quality gate (max 3 retries)
   - Apply solution and run tests (pytest/cargo/jest)
   - Collect comprehensive metrics
   - Clean up workspace
4. Generate summary report with:
   - Resolution rate (target: 70%+)
   - Quality gate pass rate (target: 85%+)
   - Average retries, wall time, tokens, cost
   - Issue type breakdown

### Output

Results are saved to: `/tmp/arkavo-phase3-{uuid}/phase3-metrics.json`

Example output:
```
╔══════════════════════════════════════════════════════════════╗
║                    PHASE 3 RESULTS SUMMARY                   ║
╠══════════════════════════════════════════════════════════════╣
║  Total Instances:             10                            ║
║  Resolved:                     7 (70%)                      ║
║  Failed:                       3                            ║
║                                                              ║
║  Avg Wall Time:            45000 ms                         ║
║  Total Cost:             $  0.50                           ║
║  Avg Cost/Instance:      $0.0500                           ║
║                                                              ║
║  Quality Gate Pass:        85.0%                           ║
║  Avg Quality Retries:       1.2                            ║
╚══════════════════════════════════════════════════════════════╝
```

## Success Criteria

**PASS if ALL criteria met**:
- ✅ 70%+ resolution rate (7+ of 10 resolved)
- ✅ 85%+ quality gate pass rate
- ✅ Average time <90s per instance
- ✅ Cost <$0.01 per instance
- ✅ No crashes or fatal errors

**INVESTIGATE if ANY fail**:
- Resolution rate <70% → Analyze error_message in metrics
- Quality gate <85% → Check issue_type_breakdown
- Time >90s → Optimize context building or file search
- Cost >$0.01 → Token usage too high, reduce context
- Crashes → Check logs for stack traces

## Known Limitations

### Current Implementation
- **No iterative refinement**: Single-shot solution generation
- **No test feedback loop**: Doesn't retry based on test failures
- **Static context**: Doesn't adapt search based on initial results
- **Sequential execution**: One instance at a time (can be parallelized)

### Test Framework Support
- ✅ pytest (Python)
- ✅ cargo test (Rust)
- ✅ jest/npm test (JavaScript)
- ❌ Other frameworks default to generic PASS/FAIL detection

## After Execution

### 1. Analyze Results

```bash
# View full metrics
cat /tmp/arkavo-phase3-*/phase3-metrics.json | jq '.'

# Extract key metrics
cat /tmp/arkavo-phase3-*/phase3-metrics.json | jq '.summary | {
  resolution_rate: .resolved_percentage,
  quality_gate_pass: .quality_gate_pass_rate,
  avg_retries: .avg_quality_retries,
  avg_time_ms: .avg_wall_time_ms,
  total_cost: .total_cost_usd
}'

# View failed instances
cat /tmp/arkavo-phase3-*/phase3-metrics.json | jq '.instances[] | select(.resolved == false) | {
  instance_id,
  error_message,
  issue_type
}'
```

### 2. Generate Documentation

Create `docs/phase3-results-{date}.md` with:
- Executive summary (PASS/FAIL against criteria)
- Detailed metrics table
- Comparison vs Phase 1 baseline
- Issue type analysis
- Recommendations for improvement

### 3. Update Issue & PR

Update GitHub Issue #350 and PR #351 with:
- Final resolution rate
- Quality gate effectiveness
- Key findings
- Next steps (Phase 4: Scale-up or optimization)

## Troubleshooting

### Issue: "No relevant files found"
**Cause**: Repository clone failed or file search not working
**Debug**:
```bash
# Check if repos are being cloned
ls -la /tmp/arkavo-phase3-*/instance-*/

# Enable debug logging
RUST_LOG=arkavo_orchestrator=debug cargo run --example swe-bench-arkavo-phase3
```

### Issue: "Quality gate failed"
**Cause**: ResponseJudge detected issues (hallucinated tools, invalid params, etc.)
**Expected**: This is normal, should trigger retry with model escalation
**Check**: metrics.quality_retries should be >0, issue_type should show reason

### Issue: "Patch application failed"
**Cause**: Solution doesn't generate valid git diff
**Expected**: Some failures normal (complex problems)
**Check**: metrics.error_message for specific git apply errors

### Issue: Tests timeout
**Cause**: Long-running test suites
**Expected**: 300s timeout per instance
**Solution**: Increase timeout in SolutionApplier or skip expensive tests

## Optimization Opportunities

### If Resolution Rate <70%
1. **Improve context relevance**: Better keyword extraction from problem statement
2. **Increase context size**: Raise max_context_tokens from 8000 to 12000
3. **Add iterative refinement**: Retry with test failure feedback
4. **Better file selection**: Use AST-based relevance scoring instead of keyword density

### If Quality Gate <85%
1. **Tune ResponseJudge**: Adjust prompts or use larger judge model
2. **Add validation**: Check solution before sending to judge
3. **Model selection**: Start with stronger model (Gemini Pro instead of Flash)

### If Time >90s
1. **Parallel file search**: Use rayon for concurrent file reads
2. **Cache contexts**: Save analyzed contexts for similar problems
3. **Faster cloning**: Use shallow clones (git clone --depth 1)
4. **Skip large files**: Limit file size for context inclusion

### If Cost >$0.01
1. **Reduce context**: More aggressive truncation (6000 tokens instead of 8000)
2. **Smarter file selection**: Include only top 5 files instead of 10
3. **Use cheaper models**: Gemini Flash instead of Pro for generation
4. **Cache expensive operations**: Don't re-analyze same repos

## Next Phase

### Phase 4: Scale & Optimize (Future)
Once Phase 3 validates 70%+ resolution:
1. **Scale to 50+ instances** for statistical significance
2. **A/B test optimizations** (different context sizes, models, strategies)
3. **Iterative refinement** with test feedback loops
4. **Multi-model ensemble** (combine predictions from multiple models)
5. **Semantic caching** of analyzed code contexts
6. **Public leaderboard** comparison with other SWE-bench approaches

### Phase 5: Production Deployment (Future)
1. **GitHub webhook integration** for real-time issue resolution
2. **Cost optimization** with budget constraints
3. **Human-in-the-loop** for review and approval
4. **Continuous benchmarking** on new SWE-bench releases

## Files & References

### Key Files
- `crates/arkavo-bench/examples/swe-bench-arkavo-phase3.rs` - Test runner
- `crates/arkavo-bench/src/arkavo_mode.rs` - Arkavo-assisted mode integration
- `crates/arkavo-orchestrator/src/code_solver.rs` - Context building & solution generation
- `crates/arkavo-bench/src/solution_applier.rs` - Patch application & test execution
- `crates/arkavo-context/src/prompt_enricher.rs` - Prompt formatting

### Documentation
- `docs/phase3-execution-log.md` - Issues found and fixes applied
- `docs/arkavo-vs-raw-llm-comparison.md` - Expected improvements
- `crates/arkavo-bench/README.md` - API documentation

### Related Issues
- Issue #350: GitHub Issue Resolution Integration
- PR #351: Arkavo-Assisted Benchmarking Implementation

## Quick Commands Reference

```bash
# Build Phase 3 runner
cargo build -p arkavo-bench --example swe-bench-arkavo-phase3

# Run 1 instance (quick test)
GEMINI_API_KEY=<key> NUM_INSTANCES=1 cargo run --example swe-bench-arkavo-phase3

# Run 10 instances (full Phase 3)
GEMINI_API_KEY=<key> NUM_INSTANCES=10 cargo run --example swe-bench-arkavo-phase3

# View metrics
cat /tmp/arkavo-phase3-*/phase3-metrics.json | jq '.summary'

# Check specific instance
cat /tmp/arkavo-phase3-*/phase3-metrics.json | jq '.instances[0]'

# Count resolved
cat /tmp/arkavo-phase3-*/phase3-metrics.json | jq '.instances | map(select(.resolved)) | length'
```

## Gemini API Key

The user provided: `REDACTED_API_KEY`

**DO NOT COMMIT THIS KEY**. Use it only for testing, export as environment variable.

---

**Ready to execute!** All infrastructure is in place, bugs are fixed, code is committed and pushed. Just run the commands above to validate Phase 3 targets.
