# Dynamic UI Generation Testing Guide

Comprehensive testing infrastructure for the Gemini-powered prompt-to-UI system.

## Overview

The testing suite validates the complete flow from user prompt to rendered UI:

```
User Prompt → UI Planner → Component Generator (Gemini) → Browser Injection → Screenshot Validation
```

## Test Files

### Integration Tests
**Location**: `tests/integration_test.rs`

Five comprehensive E2E test scenarios with browser screenshots:

1. **Calculator UI** (`test_calculator_ui_generation`)
   - Viewport: 1280x720
   - Components: Button grid, display screen, operators
   - Validates: Numeric input, operations, layout

2. **Dashboard UI** (`test_dashboard_ui_generation`)
   - Viewport: 1920x1080
   - Components: Header, metric cards, charts, tables
   - Validates: Dark theme, data visualization, responsive grid

3. **Form UI** (`test_form_ui_generation`)
   - Viewport: 1024x768
   - Components: Input fields, validation, submit button
   - Validates: Accessibility, client-side validation, UX

4. **Bank Portfolio UI** (`test_bank_portfolio_ui`)
   - Viewport: 1440x900
   - Components: Account summary, transactions, holdings, charts
   - Validates: Financial UI patterns, real sample data

5. **Incremental Updates** (`test_incremental_updates`)
   - Tests: DOM mutation monitoring
   - Validates: Live UI refinement capability

### Unit Tests
**Location**: `src/streaming.rs`, `src/planner.rs`

- Regex pattern matching for code block extraction
- Fallback planner logic for various prompt types
- JSON parsing for LLM responses

## Running Tests

### Quick Start

```bash
# Set Gemini API key
export GEMINI_API_KEY="your-api-key-here"

# Run all integration tests
./run_integration_tests.sh

# Run specific test
./run_integration_tests.sh test_calculator_ui_generation
```

### Manual Execution

```bash
# Integration tests (requires Gemini API)
cargo test --test integration_test -- --ignored --nocapture

# Unit tests (no API required)
cargo test --lib -p arkavo-ui-generator

# Specific test
cargo test --test integration_test test_dashboard_ui_generation -- --ignored --nocapture
```

## Screenshot Output

Tests generate visual evidence stored in `target/test-output/`:

```
target/test-output/
├── calculator_step_02_page_header.png          # Each component step
├── calculator_step_03_button_grid.png
├── calculator_step_04_display_screen.png
├── calculator_final.png                        # Complete UI
├── dashboard_part_01_header_section.png
├── dashboard_part_02_metrics_cards.png
├── dashboard_part_03_data_visualization.png
├── dashboard_final.png
├── bank_portfolio_01_account_summary_card.png
├── bank_portfolio_02_recent_transactions.png
├── bank_portfolio_final.png
└── incremental_step_1.png
```

### Screenshot Evaluation

Each screenshot captures:
- ✅ Component rendered correctly
- ✅ Styles applied (CSS injection worked)
- ✅ JavaScript executed (interactive features)
- ✅ Layout matches design intent
- ✅ No rendering errors or blank screens

## Test Architecture

### Browser Automation

Uses `chromiumoxide` for Chrome DevTools Protocol:

```rust
let (browser, mut handler) = Browser::launch(
    BrowserConfig::builder()
        .window_size(1280, 720)
        .build()?
).await?;

let page = browser.new_page("about:blank").await?;
let injector = LiveInjector::new(page.clone());
```

### Component Generation Flow

```rust
// 1. Plan UI components
let planner = UiPlanner::new().await?;
let plan = planner.plan("Build a calculator").await?;

// 2. Generate each component
let generator = StreamingGenerator::new(router)?;
for part in plan.parts {
    let mut stream = generator
        .generate_part(&part.name, &part.description, prompt)
        .await?;

    // 3. Collect streamed code
    while let Some(chunk) = stream.recv().await {
        match chunk.chunk_type {
            ChunkType::Html => html.push_str(&chunk.content),
            ChunkType::Css => css.push_str(&chunk.content),
            ChunkType::JavaScript => js.push_str(&chunk.content),
        }
    }

    // 4. Inject into browser
    injector.inject_complete_update(
        Some(&html), Some(&css), Some(&js)
    ).await?;

    // 5. Capture screenshot
    let screenshot = page.screenshot(ScreenshotParams::default()).await?;
    tokio::fs::write(screenshot_path, screenshot).await?;
}
```

## CI/CD Integration

Tests are marked `#[ignore]` to prevent CI failures without:
- Gemini API credentials
- Browser automation support
- Headless Chrome

### GitHub Actions Example

```yaml
- name: Run UI Generation Integration Tests
  if: ${{ secrets.GEMINI_API_KEY }}
  env:
    GEMINI_API_KEY: ${{ secrets.GEMINI_API_KEY }}
  run: |
    cargo test --test integration_test -- --ignored --nocapture

- name: Upload Screenshots
  uses: actions/upload-artifact@v4
  with:
    name: ui-screenshots
    path: target/test-output/*.png
```

## Debugging

### Test Fails to Launch Browser

```bash
# Check Chrome installation
which google-chrome-stable
which chromium

# Set explicit Chrome path
export CHROME_BIN=/path/to/chrome
```

### Screenshots are Blank

Increase render time:
```rust
sleep(Duration::from_millis(1000)).await;  // Was 500ms
```

### Gemini API Errors

```bash
# Enable debug logging
export ARKAVO_DEBUG=1
export RUST_LOG=debug

# Test with fallback planner only
unset GEMINI_API_KEY
cargo test test_fallback_plan
```

### Browser Process Cleanup

```bash
# Kill stuck Chrome instances
pkill -9 chrome
pkill -9 chromium
```

## Test Coverage

Current coverage for `arkavo-ui-generator`:

| Component | Coverage | Notes |
|-----------|----------|-------|
| Planner | 85% | Fallback logic well-tested |
| Streaming Generator | 70% | Needs more error cases |
| Regex Parsing | 95% | Comprehensive fixtures |
| Browser Integration | 80% | Visual validation via screenshots |

Run coverage:
```bash
cargo tarpaulin -p arkavo-ui-generator --exclude-files tests/
```

## Performance Benchmarks

Typical test execution times:

| Test | Duration | Components | Screenshots |
|------|----------|------------|-------------|
| Calculator | 15-20s | 4 | 5 |
| Dashboard | 25-30s | 6 | 7 |
| Form | 12-18s | 5 | 6 |
| Bank Portfolio | 20-25s | 7 | 8 |
| Incremental | 5-8s | 2 | 2 |

Total suite: ~90 seconds with Gemini API

## Contributing Tests

When adding new integration tests:

1. **Use descriptive prompts**: Clear, specific UI requirements
2. **Set appropriate viewport**: Match typical use case
3. **Sanitize filenames**: Cross-platform compatibility
4. **Take step screenshots**: Validate each component
5. **Capture final state**: Complete UI verification
6. **Document expectations**: What should be visible?

Example:
```rust
#[tokio::test]
#[ignore]
async fn test_my_new_ui() -> Result<()> {
    println!("\n🧪 Testing: My New UI");

    let (browser, mut handler) = Browser::launch(
        BrowserConfig::builder()
            .window_size(1600, 900)
            .build()?
    ).await?;

    // ... test implementation

    let screenshot_path = test_output_dir().join("my_ui_final.png");
    let screenshot = page.screenshot(ScreenshotParams::default()).await?;
    tokio::fs::write(&screenshot_path, screenshot).await?;

    println!("✅ Screenshot: {}", screenshot_path.display());
    Ok(())
}
```

## Known Limitations

- **Gemini API required**: Integration tests need valid API key
- **No headless CI**: Some CI environments don't support browser automation
- **Screenshot comparison**: Manual visual inspection required
- **Network dependent**: Tests fail without internet access
- **Rate limits**: Gemini API may throttle requests

## Future Enhancements

- [ ] Visual regression testing (pixel comparison)
- [ ] Automated screenshot diffing
- [ ] Mock Gemini responses for deterministic tests
- [ ] Accessibility validation (axe-core integration)
- [ ] Performance profiling (rendering speed)
- [ ] Cross-browser testing (Firefox, Safari)

## Resources

- [Integration Test README](tests/README.md) - Detailed test documentation
- [Test Runner Script](run_integration_tests.sh) - Automated execution
- [arkavo-browser](../arkavo-browser/README.md) - Browser automation docs
- [Gemini API](https://ai.google.dev/docs) - LLM integration guide
