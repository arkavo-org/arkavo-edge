# Bug Report & Improvement Analysis

**Date**: 2025-10-09
**Test Suite**: Integration Tests for Dynamic UI Generation
**Status**: ✅ Tests Pass | ⚠️ Behavior Issues Found

---

## Critical Issues

### 🐛 BUG #1: UiPlanner Never Uses LLM for Planning

**File**: `crates/arkavo-ui-generator/src/planner.rs:36-44`

**Severity**: HIGH
**Impact**: Core feature not working as designed

**Description**:
The `UiPlanner.plan()` method builds a planning prompt and routes to the LLM, but **completely ignores the results** and always uses the fallback planner.

**Current Code**:
```rust
pub async fn plan(&self, user_prompt: &str) -> Result<BuildPlan> {
    let _planning_prompt = self.build_planning_prompt(user_prompt);  // ← Built but never used!

    if let Some(router) = &self.router {
        let _decision = router.route(user_prompt).await.ok();  // ← Routed but result ignored!
    }

    self.fallback_plan(user_prompt)  // ← ALWAYS returns fallback!
}
```

**Evidence**:
Test output shows:
```
UiPlanner: Generated 2 parts for prompt: 'Build a calculator'
  - Page Header (part-1): Top navigation bar with logo and menu
  - Page Footer (part-2): Footer with links, disclaimers, and copyright
```

For a calculator request, we should get components like:
- Button Grid (numeric keypad)
- Display Screen
- Operation Buttons
- Memory Functions
- Clear/Reset Controls

Instead, we got generic header/footer because the fallback doesn't recognize "calculator".

**Root Cause**:
The method signature suggests LLM-based planning was planned but never implemented. The planning prompt is constructed correctly (lines 46-67) but never sent to an LLM provider.

**Impact**:
- Generic UIs for most prompts
- Limited keyword recognition ("bank", "portfolio", "chart", "form", "dashboard")
- No calculator, todo list, chat, or other common UI patterns
- Undermines the core value proposition

---

### 🐛 BUG #2: Missing Keywords in Fallback Planner

**File**: `crates/arkavo-ui-generator/src/planner.rs:91-224`

**Severity**: MEDIUM
**Impact**: Poor fallback coverage

**Current Keywords**:
- ✅ "bank", "account" → Banking components
- ✅ "portfolio", "stock" → Investment components
- ✅ "chart", "graph" → Visualization
- ✅ "table", "list" → Data tables
- ✅ "form", "input" → Forms
- ✅ "dashboard" → Widget grids
- ❌ "calculator" → **NOT RECOGNIZED**
- ❌ "todo", "task" → **NOT RECOGNIZED**
- ❌ "chat", "message" → **NOT RECOGNIZED**
- ❌ "game", "board" → **NOT RECOGNIZED**
- ❌ "calendar", "schedule" → **NOT RECOGNIZED**

**Missing Patterns**:
```rust
// Should add:
if keywords.contains("calculator") || keywords.contains("calc") {
    // Button grid, display, operators
}

if keywords.contains("todo") || keywords.contains("task") {
    // Task list, checkboxes, add/delete controls
}

if keywords.contains("chat") || keywords.contains("message") {
    // Message list, input box, send button
}

if keywords.contains("calendar") || keywords.contains("schedule") {
    // Month/week/day views, event cells
}
```

---

### 🐛 BUG #3: StreamingGenerator Router is Optional But Not Handled

**File**: `crates/arkavo-ui-generator/src/streaming.rs:40-44, 81-84`

**Severity**: LOW
**Impact**: Inconsistent behavior

**Description**:
StreamingGenerator now accepts `Option<Arc<Router>>` but still calls `router.route()` unconditionally in the spawn block, which will panic if router is None after the recent refactor.

**Current Code**:
```rust
pub struct StreamingGenerator {
    router: Option<Arc<Router>>,  // ← Made optional
    // ...
}

// But in generate_part():
tokio::spawn(async move {
    if let Some(router_instance) = router {
        let _decision = router_instance.route(&prompt).await;  // ← Good!
    }
    // ... rest works fine
});
```

**Status**: ✅ Already fixed in recent changes
**Action**: Verify tests with `StreamingGenerator::new_without_router()`

---

## Quality Issues

### ⚠️ ISSUE #1: Unused LLM Routing Results

**Files**:
- `crates/arkavo-ui-generator/src/planner.rs:40`
- `crates/arkavo-ui-generator/src/streaming.rs:83`

**Description**:
Both files route prompts to the LLM but don't use the routing decision:

```rust
let _decision = router.route(&prompt).await;  // ← Underscore prefix indicates "intentionally unused"
```

**Impact**:
- Wastes local LLM inference cycles
- Doesn't benefit from cost-aware routing
- Misses opportunities for model selection (local vs. remote)

**Questions**:
1. Should the decision influence which provider generates components?
2. Should complex UIs use Gemini while simple ones use local Gemma?
3. Is routing meant to log metrics only?

---

### ⚠️ ISSUE #2: Test Generated Wrong Components

**Test**: `test_calculator_ui_generation`
**Expected**: Calculator button grid, display screen
**Actual**: Generic header and footer

**Analysis**:
This is a consequence of BUG #1 (no LLM planning) and BUG #2 (missing "calculator" keyword).

**User Experience Impact**:
If a user runs:
```bash
cargo run --bin arkavo -- ui --blank --prompt "Build a calculator"
```

They will get:
- ❌ Navigation header with "Calculator" logo
- ❌ Footer with disclaimer text
- ❌ **No actual calculator functionality**

This is a poor first impression and doesn't demonstrate the system's capabilities.

---

## Performance Observations

### ✅ POSITIVE: Local Model Working

**Evidence**:
```
Loading model 'gemma-3-270m-it' on device Cpu
Detected GGUF architecture: gemma3
Loading Gemma 3 GGUF model...
Successfully loaded gemma3 model
```

**Impact**:
- HuggingFace repo ID resolution working (`resolve_hf_repo_to_path()`)
- Local Gemma 3 270M loads successfully
- Router initialization no longer fails
- Hybrid local + remote LLM architecture functional

### 📊 Test Metrics

| Metric | Value | Assessment |
|--------|-------|------------|
| Test Duration | 21.36s | ✅ Acceptable |
| Components Generated | 2 | ⚠️ Too few for calculator |
| Screenshots Captured | 3 | ✅ Working |
| Local Model Load Time | ~2s | ✅ Fast |
| Browser Automation | Success | ✅ Chrome working |

---

## Recommendations

### Priority 1: Fix UiPlanner to Use LLM

**File**: `crates/arkavo-ui-generator/src/planner.rs`

**Action**: Implement actual LLM-based planning

**Suggested Fix**:
```rust
pub async fn plan(&self, user_prompt: &str) -> Result<BuildPlan> {
    // Try LLM planning first
    if let Some(router) = &self.router {
        if let Ok(plan) = self.try_llm_plan(user_prompt).await {
            return Ok(plan);
        }
        eprintln!("LLM planning failed, using fallback");
    }

    // Fallback if LLM unavailable or fails
    self.fallback_plan(user_prompt)
}

async fn try_llm_plan(&self, user_prompt: &str) -> Result<BuildPlan> {
    let planning_prompt = self.build_planning_prompt(user_prompt);

    // Use LocalProvider with Gemma for fast planning
    let provider = LocalProvider::new(
        "gemma-3-270m-it".to_string(),
        Some("unsloth/gemma-3-270m-it-GGUF".to_string()),
    )?;
    provider.initialize().await?;

    let response = provider.complete(vec![Message {
        role: "user".to_string(),
        content: planning_prompt,
    }]).await?;

    self.parse_plan(&response)
}
```

**Estimated Effort**: 2-3 hours
**Test Coverage**: Add test for LLM planning path

---

### Priority 2: Expand Fallback Keywords

**File**: `crates/arkavo-ui-generator/src/planner.rs:91-224`

**Action**: Add common UI patterns

**Suggested Additions**:
```rust
// Calculator
if keywords.contains("calculator") || keywords.contains("calc") {
    parts.push(ComponentPart {
        id: format!("part-{part_id}"),
        name: "Calculator Display".to_string(),
        description: "Numeric display showing current value and operations".to_string(),
        priority: part_id,
    });
    part_id += 1;

    parts.push(ComponentPart {
        id: format!("part-{part_id}"),
        name: "Number Pad".to_string(),
        description: "Grid of buttons for digits 0-9".to_string(),
        priority: part_id,
    });
    part_id += 1;

    parts.push(ComponentPart {
        id: format!("part-{part_id}"),
        name: "Operation Buttons".to_string(),
        description: "Buttons for +, -, *, /, =, and clear".to_string(),
        priority: part_id,
    });
    part_id += 1;
}

// Todo List
if keywords.contains("todo") || keywords.contains("task") {
    parts.push(ComponentPart {
        id: format!("part-{part_id}"),
        name: "Task Input".to_string(),
        description: "Input field and add button for new tasks".to_string(),
        priority: part_id,
    });
    part_id += 1;

    parts.push(ComponentPart {
        id: format!("part-{part_id}"),
        name: "Task List".to_string(),
        description: "Scrollable list of tasks with checkboxes and delete buttons".to_string(),
        priority: part_id,
    });
    part_id += 1;
}

// Chat
if keywords.contains("chat") || keywords.contains("message") {
    parts.push(ComponentPart {
        id: format!("part-{part_id}"),
        name: "Message Thread".to_string(),
        description: "Scrollable conversation history with user/assistant messages".to_string(),
        priority: part_id,
    });
    part_id += 1;

    parts.push(ComponentPart {
        id: format!("part-{part_id}"),
        name: "Message Input".to_string(),
        description: "Text input with send button for new messages".to_string(),
        priority: part_id,
    });
    part_id += 1;
}
```

**Estimated Effort**: 1 hour
**Test Coverage**: Update `test_fallback_plan()` with calculator test case

---

### Priority 3: Use Routing Decisions

**Files**:
- `crates/arkavo-ui-generator/src/planner.rs`
- `crates/arkavo-ui-generator/src/streaming.rs`

**Action**: Make routing decisions actionable

**Options**:
1. **Log Only**: Keep current behavior, just for metrics
2. **Model Selection**: Route complex UIs to Gemini, simple to local Gemma
3. **Cost Optimization**: Use decision for budget-aware generation

**Suggested Fix** (Model Selection):
```rust
// In streaming.rs
tokio::spawn(async move {
    let use_gemini = if let Some(router_instance) = router {
        router_instance.route(&prompt).await
            .map(|d| d.recommended_model.is_cloud())
            .unwrap_or(true)  // Default to Gemini if routing fails
    } else {
        true  // No router = always use Gemini
    };

    if use_gemini && std::env::var("GEMINI_API_KEY").is_ok() {
        // Use Gemini (current path)
    } else {
        // Use local Gemma for generation
    }
});
```

**Estimated Effort**: 3-4 hours
**Test Coverage**: Test both local and remote generation paths

---

## Test Improvements

### Add Calculator-Specific Test

**File**: `crates/arkavo-ui-generator/tests/integration_test.rs`

**Add Test**:
```rust
#[tokio::test]
#[ignore]
async fn test_calculator_has_required_components() -> Result<()> {
    let planner = UiPlanner::new().await?;
    let plan = planner.plan("Build a calculator").await?;

    // Should have calculator-specific components
    let component_names: Vec<_> = plan.parts.iter()
        .map(|p| p.name.to_lowercase())
        .collect();

    assert!(
        component_names.iter().any(|n|
            n.contains("button") ||
            n.contains("pad") ||
            n.contains("keypad") ||
            n.contains("number")
        ),
        "Calculator should have button/keypad component, got: {:?}",
        component_names
    );

    assert!(
        component_names.iter().any(|n|
            n.contains("display") ||
            n.contains("screen")
        ),
        "Calculator should have display component, got: {:?}",
        component_names
    );

    Ok(())
}
```

---

## Summary

### Critical Path
1. **Fix UiPlanner LLM integration** (BUG #1) - Blocks core functionality
2. **Add calculator keywords** (BUG #2) - Quick win for testing
3. **Re-run integration tests** - Verify fixes

### Success Criteria
- ✅ Calculator test generates button grid + display
- ✅ LLM planning used when available
- ✅ Fallback covers top 10 UI patterns
- ✅ Screenshots show actual calculator UI

### Risk Assessment
**LOW RISK**: Changes are isolated to planner.rs, existing fallback remains intact

---

**Generated by**: Integration Test Analysis
**Next Steps**: Review with team, prioritize fixes, create GitHub issues
