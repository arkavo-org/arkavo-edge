# Dynamic UI Generation - Status Report

**Date**: 2025-10-09
**Branch**: `feature/dynamic-ui-generation`
**Last Commit**: `b5d4f47` - Refactor UiPlanner to accept existing Router instance

---

## 🎯 Project Goal

Enable arkavo to generate UI components dynamically from natural language prompts using:
- Local Gemma 3 270M for planning (breaking down UI into components)
- Gemini API for code generation (HTML/CSS/JavaScript)
- Live browser injection for real-time rendering

---

## ✅ What's Working

### 1. **Chat Command** ✅
```bash
cargo run --bin arkavo -- chat --prompt "hi"
```
- Local Gemma 3 270M loads successfully
- Tokenizer works correctly
- Generates responses at 42-88 tok/s
- **This proves the tokenizer CAN be loaded properly**

### 2. **Architecture Improvements** ✅
- Removed fallback planner (all components now come from LLM)
- Refactored UiPlanner to accept Router instance (prevents duplicate model loading)
- Router only initializes once and is reused
- Added `Router::get_local_provider()` for LLM completions
- Integration tests updated and passing (when run with `--ignored`)

### 3. **Browser Automation** ✅
- chromiumoxide integration working
- Screenshot capture functional
- Live injection of HTML/CSS/JavaScript working
- Test output directory created at `target/test-output/`

### 4. **Gemini Integration** ✅
- Streaming API working
- Environment variables properly configured
- Code generation functional (when planning works)

---

## ❌ What's Broken

### **CRITICAL ISSUE: Tokenizer Not Loading in UI Command**

**Error**:
```
Loading model 'gemma-3-270m-it' on device Metal(MetalDevice(DeviceId(1)))
Detected GGUF architecture: gemma3
Loading Gemma 3 GGUF model...
Successfully loaded gemma3 model
AG-UI: Error auto-submitting initial prompt: LLM planning failed: Classification error: LLM completion failed: Model error: Tokenizer not loaded
```

**Symptoms**:
1. Model loads successfully ✅
2. Only loads once (no duplicate loading) ✅
3. But tokenizer fails to initialize ❌
4. `arkavo ui --blank --prompt "pet finder"` fails immediately
5. `arkavo chat --prompt "hi"` works perfectly

**Root Cause Analysis**:

The issue is in `/Users/paul/Projects/arkavo/arkavo-edge/crates/arkavo-llm/src/local/model_loader.rs`

**Tokenizer Loading Flow** (lines 328-362):
```rust
fn try_load_tokenizer(&mut self, content: &candle_core::quantized::gguf_file::Content) {
    // 1. Try embedded tokenizer from GGUF metadata
    if self.try_load_embedded_tokenizer(&content.metadata) {
        return;
    }

    // 2. Try to construct tokenizer from metadata
    if self.try_construct_tokenizer_from_metadata(content) {
        return;
    }

    // 3. For Gemma models, try HuggingFace cache
    if self.model_name.starts_with("gemma") && self.try_load_from_hf_cache() {
        return;
    }

    // 4. Fallback: look for tokenizer alongside .gguf file
    if self.try_load_alongside_model() {
        return;
    }

    // All methods failed!
    eprintln!("WARNING: Could not load tokenizer for {}", self.model_name);
}
```

**The Problem**:
- Method #1 (`try_load_embedded_tokenizer`) fails - GGUF doesn't have embedded tokenizer
- Method #2 (`try_construct_tokenizer_from_metadata`) creates a **STUB tokenizer** (lines 527-563)
- This stub has no vocabulary! It's just `BPE::default()` with no actual token mappings
- Method #3 and #4 never get called because method #2 returns `true` even though it created a broken stub

**Evidence from Code** (`model_loader.rs:552-563`):
```rust
fn create_tokenizer_stub(&self) -> Option<Arc<tokenizers::Tokenizer>> {
    use tokenizers::models::bpe::BPE;
    use tokenizers::tokenizer::Tokenizer;

    // Create a minimal BPE tokenizer
    // In production, this would load actual vocabulary and merges  ← COMMENT ADMITS IT'S INCOMPLETE!
    let bpe = BPE::default();  // ← NO VOCABULARY!
    let tokenizer = Tokenizer::new(bpe);

    Some(Arc::new(tokenizer))
}
```

**Why Chat Works But UI Doesn't**:
- Need to investigate initialization paths
- Likely `ProviderFactory` (used by chat) has different tokenizer loading
- Or chat uses a different model path that includes tokenizer files

---

## 🔧 Files Modified

### Core Implementation:
1. `crates/arkavo-ui-generator/src/planner.rs` - LLM-based planning (removed fallback)
2. `crates/arkavo-ui-generator/src/streaming.rs` - Streaming code generation
3. `crates/arkavo-ui-generator/src/ui_handler.rs` - UI state management
4. `crates/arkavo-agui/src/gateway.rs` - WebSocket event handling
5. `crates/arkavo-router/src/lib.rs` - Added `get_local_provider()` method
6. `crates/arkavo-router/src/classifier.rs` - Added `complete()` method for direct LLM access

### Testing:
7. `crates/arkavo-ui-generator/tests/integration_test.rs` - E2E browser tests
8. `crates/arkavo-ui-generator/BUG_REPORT.md` - Detailed analysis of issues found

### Documentation:
9. `AGENTS.md` - Updated with UI generation context
10. `crates/arkavo-ui-generator/STATUS.md` - This file

---

## 📸 Test Results

### Screenshots Generated (when LLM planning worked):
Located in `crates/arkavo-ui-generator/target/test-output/`:
- `calculator_final.png` - Final calculator UI (showing header/footer from old fallback)
- `calculator_step_02_page_header.png` - Step-by-step generation
- `calculator_step_03_page_footer.png`
- `incremental_step_1.png` - Incremental update test
- `incremental_step_2.png`

**Note**: These screenshots show the OLD fallback planner results (generic header/footer).
After removing fallback, we can't generate new screenshots until tokenizer issue is fixed.

---

## 🚀 Next Steps (Monday Follow-up)

### **Priority 1: Fix Tokenizer Loading** 🔴

**Option A - Load Real Tokenizer from HuggingFace Cache**:
1. Debug why `try_load_from_hf_cache()` isn't finding tokenizer
2. Check what files exist in `~/.cache/huggingface/hub/models--unsloth--gemma-3-270m-it-GGUF/`
3. Look for `tokenizer.json`, `tokenizer_config.json`, or `.spm` files
4. Fix path resolution if files exist

**Option B - Extract Tokenizer from GGUF**:
1. Investigate `try_load_embedded_tokenizer()` more deeply
2. Check if GGUF metadata has tokenizer data in different format
3. Look at how other tools extract tokenizers from GGUF files
4. Reference: candle examples for GGUF tokenizer extraction

**Option C - Compare Chat vs UI Initialization**:
1. Add debug logging to both paths
2. Run both commands with `RUST_LOG=debug`
3. Compare tokenizer initialization sequence
4. Find where they diverge

**Commands to Debug**:
```bash
# Working case
RUST_LOG=debug cargo run --bin arkavo -- chat --prompt "hi" 2>&1 | grep -i token

# Broken case
RUST_LOG=debug cargo run --bin arkavo -- ui --blank --prompt "test" 2>&1 | grep -i token
```

**Fix Location**: `crates/arkavo-llm/src/local/model_loader.rs`
- Remove or fix `create_tokenizer_stub()` (lines 552-563)
- Make `try_construct_tokenizer_from_metadata()` actually work
- Or ensure `try_load_from_hf_cache()` succeeds

---

### **Priority 2: Test Full Flow** 🟡

Once tokenizer is fixed:

1. **Manual Test**:
```bash
export GEMINI_API_KEY='<your-api-key>'
cargo run --bin arkavo -- ui --blank --prompt "pet finder"
```

Expected behavior:
- ✅ Local Gemma 3 270M loads
- ✅ Tokenizer initializes
- ✅ UI plan generated (5-10 components)
- ✅ Each component code generated via Gemini
- ✅ Live rendering in browser

2. **Integration Tests**:
```bash
export GEMINI_API_KEY='<your-api-key>'
cargo test --test integration_test -- --ignored --nocapture
```

3. **Screenshot Validation**:
- Check `target/test-output/` for new screenshots
- Verify components look reasonable
- Verify incremental updates work

---

### **Priority 3: Clean Up** 🟢

1. **Remove Test Artifacts**:
```bash
# These were accidentally committed
git rm crates/arkavo-ui-generator/target/test-output/*.png
git rm Cargo.toml.demo
git rm crates/arkavo-mcp-tools/src/browser.rs  # If unused
git rm examples/demo_incremental_ui.rs  # If demo only
```

2. **Update Documentation**:
- Move `BUG_REPORT.md` content to STATUS.md
- Delete BUG_REPORT.md
- Update README with usage examples

3. **Code Quality**:
```bash
cargo clippy --fix --allow-dirty
cargo fmt
cargo test
```

---

## 📋 Verification Checklist

Before marking this feature complete:

- [ ] Tokenizer loads successfully in UI command
- [ ] `arkavo ui --blank --prompt "calculator"` generates actual calculator UI
- [ ] `arkavo ui --blank --prompt "pet finder"` generates search interface
- [ ] Integration tests pass without `--ignored` flag
- [ ] Screenshots show real UI components (not generic header/footer)
- [ ] No duplicate model loading (verify with logs)
- [ ] Router reused across multiple prompts
- [ ] Memory usage stable (no leaks from model reloading)
- [ ] Code passes `cargo clippy` without warnings
- [ ] Documentation updated with examples

---

## 🔍 Debugging Resources

### Key Files to Review:
1. `crates/arkavo-llm/src/local/model_loader.rs:328-563` - Tokenizer loading logic
2. `crates/arkavo-llm/src/local/provider.rs:115-132` - LocalProvider initialization
3. `crates/arkavo-router/src/classifier.rs:105-117` - TaskClassifier initialization
4. `crates/arkavo-llm/src/providers/factory.rs:467-477` - ProviderFactory (used by chat)

### Useful Commands:
```bash
# Check HuggingFace cache
ls -la ~/.cache/huggingface/hub/models--unsloth--gemma-3-270m-it-GGUF/snapshots/*/

# Debug tokenizer loading
RUST_LOG=arkavo_llm=debug cargo run --bin arkavo -- ui --blank --prompt "test"

# Compare working vs broken
diff <(RUST_LOG=debug cargo run --bin arkavo -- chat --prompt "hi" 2>&1) \
     <(RUST_LOG=debug cargo run --bin arkavo -- ui --blank --prompt "hi" 2>&1)

# Check if GGUF has tokenizer metadata
cargo install gguf-tools  # If available
gguf-info ~/.cache/huggingface/hub/models--unsloth--gemma-3-270m-it-GGUF/snapshots/*/*.gguf
```

---

## 🎓 What We Learned

1. **Always reuse Router instances** - Creating new ones loads models multiple times
2. **Tokenizer stubs are dangerous** - They silently fail at runtime
3. **Integration tests need screenshots** - Visual validation is critical for UI generation
4. **HuggingFace cache structure** - `models--owner--repo/snapshots/<hash>/*.gguf`
5. **GGUF metadata** - Can contain embedded tokenizers, but format varies
6. **Chat vs UI paths differ** - Different initialization sequences can cause different behavior

---

## 💬 Questions for Monday

1. **Should we bundle tokenizer files?** - Ship tokenizer.json with the binary?
2. **Should we support offline mode?** - Generate UI without Gemini API?
3. **What about model size?** - Is 270M too small for complex UI planning?
4. **Error handling strategy?** - Fail fast or graceful degradation?
5. **Caching strategy?** - Should we cache generated component code?

---

## 📞 Contact

For questions or updates:
- **Branch**: `feature/dynamic-ui-generation`
- **Last Working Commit**: `b5d4f47`
- **Blocker**: Tokenizer initialization in `model_loader.rs`
- **ETA for Fix**: Monday (investigate 3 options above)

---

**Generated**: 2025-10-09
**Status**: ⚠️ Blocked on tokenizer loading issue
**Next Review**: Monday
