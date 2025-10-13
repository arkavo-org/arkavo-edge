# UI Generator Integration Tests

This directory contains comprehensive integration tests for the dynamic UI generation feature.

## Test Overview

The integration tests validate the end-to-end flow:
1. **UI Planning**: Breaking down prompts into component parts
2. **Component Generation**: Using Gemini LLM to generate HTML/CSS/JavaScript
3. **Browser Rendering**: Injecting code into live Chrome instances
4. **Screenshot Validation**: Capturing visual output at each step

## Running Tests

### Prerequisites

- **Gemini API Key**: Required for LLM-based generation
  ```bash
  export GEMINI_API_KEY="your-api-key-here"
  ```

- **Chrome/Chromium**: Automatically launched by chromiumoxide

### Execute All Integration Tests

```bash
# Run all integration tests (they are ignored by default)
cargo test --test integration_test -- --ignored --nocapture

# Run specific test
cargo test --test integration_test test_calculator_ui_generation -- --ignored --nocapture
cargo test --test integration_test test_dashboard_ui_generation -- --ignored --nocapture
cargo test --test integration_test test_bank_portfolio_ui -- --ignored --nocapture
```

### Test Output

Screenshots are saved to: `target/test-output/`

Each test produces:
- **Step-by-step screenshots**: One per component generation
- **Final screenshot**: Complete rendered UI

Example output:
```
target/test-output/
├── calculator_step_02_page_header.png
├── calculator_step_03_button_grid.png
├── calculator_step_04_display_screen.png
├── calculator_final.png
├── dashboard_part_01_header_section.png
├── dashboard_part_02_metrics_cards.png
├── dashboard_final.png
└── bank_portfolio_final.png
```

## Test Scenarios

### 1. Calculator UI (`test_calculator_ui_generation`)
- **Prompt**: "Build a calculator"
- **Components**: Button grid, display screen, operators
- **Viewport**: 1280x720
- **Expected**: Functional calculator interface with numeric keypad

### 2. Dashboard UI (`test_dashboard_ui_generation`)
- **Prompt**: "Build a dashboard with charts and metrics"
- **Components**: Header, metric cards, charts, data tables
- **Viewport**: 1920x1080
- **Expected**: Dark-themed dashboard with visualizations

### 3. Form UI (`test_form_ui_generation`)
- **Prompt**: "Build a user registration form with validation"
- **Components**: Input fields, validation, submit button
- **Viewport**: 1024x768
- **Expected**: Accessible form with client-side validation

### 4. Bank Portfolio UI (`test_bank_portfolio_ui`)
- **Prompt**: "Build a bank account and stock portfolio page"
- **Components**: Account summary, transactions, holdings, charts
- **Viewport**: 1440x900
- **Expected**: Financial dashboard with realistic sample data

### 5. Incremental Updates (`test_incremental_updates`)
- **Test**: DOM change monitoring and incremental injection
- **Expected**: Captured mutation events for UI refinement

## What Tests Validate

✅ **Functional**:
- UI planner generates appropriate components
- Gemini API generates valid HTML/CSS/JavaScript
- Code injection works without errors
- Components render visually

✅ **Visual**:
- Screenshots capture each generation step
- Final UI matches prompt intent
- No rendering errors or blank screens

✅ **Error Handling**:
- Graceful fallback when Gemini unavailable
- Malformed responses don't crash the system
- Empty components are handled

## Debugging Failed Tests

### Test hangs or times out
```bash
# Check if Chrome process is stuck
ps aux | grep chrome

# Kill stuck processes
pkill -9 chrome
```

### "Failed to launch browser" error
- Ensure Chrome/Chromium is installed
- Check system permissions for browser automation
- Try running in headless mode (default)

### Screenshots are blank
- Verify CSS injection is working
- Check browser console for JavaScript errors
- Increase sleep duration for rendering

### Gemini API errors
- Verify `GEMINI_API_KEY` is set correctly
- Check API rate limits and quotas
- Review fallback planner is working

## CI/CD Integration

Tests are marked `#[ignore]` to avoid CI failures when:
- Gemini API key is not available
- Browser automation is not supported
- Headless Chrome is not installed

To run in CI with proper setup:
```yaml
- name: Run UI Generation Tests
  env:
    GEMINI_API_KEY: ${{ secrets.GEMINI_API_KEY }}
  run: |
    cargo test --test integration_test -- --ignored --nocapture
```

## Contributing

When adding new integration tests:
1. Follow the existing test structure
2. Use descriptive prompts
3. Take screenshots at each step
4. Document expected outcomes
5. Add sanitized filenames for cross-platform compatibility

## Troubleshooting

### Missing Screenshots
If screenshots aren't being saved:
```bash
# Check test output directory
ls -la target/test-output/

# Manually create if needed
mkdir -p target/test-output
```

### Screenshot Quality
Adjust viewport size for different test scenarios:
```rust
BrowserConfig::builder()
    .window_size(1920, 1200)  // Larger viewport
    .build()
```

### Test Cleanup
Remove old test artifacts:
```bash
rm -rf target/test-output/*.png
```

## Resources

- [Gemini API Documentation](https://ai.google.dev/docs)
- [chromiumoxide Crate](https://docs.rs/chromiumoxide)
- [arkavo-browser Documentation](../../arkavo-browser/README.md)
