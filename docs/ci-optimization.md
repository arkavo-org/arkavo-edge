# GitHub Actions CI Optimization

## Overview

The GitHub Actions workflow has been optimized to reduce CI costs and improve build times by conditionally running platform-specific builds (macOS and Windows) only when necessary.

## How It Works

### Automatic Detection

The workflow automatically analyzes changed files in each pull request to determine if platform-specific builds are needed:

**macOS builds are triggered when:**
- Changes to `crates/arkavo-mcp-macos/` (macOS-specific crate)
- Changes to `.github/workflows/` (workflow configuration)
- Changes to `Cargo.toml` or `Cargo.lock` (dependency updates)
- Changes to core crates (`crates/arkavo-cli/`, `crates/arkavo/`)

**Windows builds are triggered when:**
- Changes to `.github/workflows/` (workflow configuration)
- Changes to `Cargo.toml` or `Cargo.lock` (dependency updates)
- Changes to core crates (`crates/arkavo-cli/`, `crates/arkavo/`)

**Platform builds are skipped when:**
- Only documentation files are changed (`*.md`, `docs/`, `README`)
- Only Linux-specific changes are made
- Only test files that don't affect platform code are changed

### Manual Override

You can force platform builds by adding labels to your pull request:

- **`build:macos`** - Force macOS build
- **`build:windows`** - Force Windows build
- **`build:all-platforms`** - Force all platform builds

To add a label to your PR:
1. Go to your pull request on GitHub
2. Click on "Labels" in the right sidebar
3. Select the appropriate label(s)

## Cost Savings

### Before Optimization
- Every PR triggered builds on all platforms
- Average cost per PR: ~240 minute-equivalents
  - Linux: ~10 minutes
  - macOS: ~20 minutes × 10x cost = 200 minute-equivalents
  - Windows: ~15 minutes × 2x cost = 30 minute-equivalents

### After Optimization
- Typical PR (no platform changes): ~10 minute-equivalents (96% reduction)
- Platform-specific PR: ~240 minute-equivalents (same as before)
- **Average savings: ~230 minute-equivalents per PR**

## Developer Experience

### What You'll See

When you open a pull request, the workflow will:

1. Run the `check-platform-changes` job to analyze your changes
2. Display which platform builds are needed in the job output
3. Skip unnecessary platform builds with a clear indication
4. Provide a tip about using labels to force builds if needed

### Example Output

```
=== Build Decision ===
macOS build needed: false
Windows build needed: false

💡 Tip: To force platform builds, add labels 'build:macos', 'build:windows', or 'build:all-platforms'
```

## Safety Measures

### Release Workflow Unchanged
The release workflow (triggered on merge to main) continues to build all platforms to ensure complete coverage for releases.

### Conservative Path Matching
The path patterns are intentionally broad to avoid false negatives. If there's any doubt, the build will run.

### Easy Override
Labels provide an easy way to force builds when needed, ensuring developers can always get platform-specific feedback.

### Platform Issues Detection
Even if platform builds are skipped in PRs, the release workflow will catch any platform-specific issues before they reach users.

## Troubleshooting

### My PR needs a platform build but it was skipped

**Solution:** Add the appropriate label to your PR:
- For macOS: Add `build:macos` label
- For Windows: Add `build:windows` label
- For both: Add `build:all-platforms` label

### I'm not sure if my changes affect a specific platform

**Solution:** Add the `build:all-platforms` label to be safe. The workflow will run all platform builds.

### The platform build detection seems incorrect

**Solution:** 
1. Check the `check-platform-changes` job output to see which files triggered the decision
2. If the detection is incorrect, please open an issue with details
3. Use labels to override the decision for your PR

## Implementation Details

### Workflow Jobs

1. **check-platform-changes**: Analyzes changed files and sets outputs for downstream jobs
2. **build-macos**: Conditionally runs based on detection or labels
3. **build-windows**: Conditionally runs based on detection or labels
4. **smoke-test-macos**: Only runs if build-macos succeeded
5. **smoke-test-windows**: Only runs if build-windows succeeded
6. **release-readiness**: Updated to handle skipped platform builds

### Path Patterns

The workflow uses regex patterns to match changed files:

```bash
# macOS triggers
'^crates/arkavo-mcp-macos/|^\.github/workflows/|^Cargo\.(toml|lock)$|^crates/arkavo-cli/|^crates/arkavo/'

# Windows triggers
'^\.github/workflows/|^Cargo\.(toml|lock)$|^crates/arkavo-cli/|^crates/arkavo/'

# Documentation-only (skip all platforms)
'\.md$|^docs/|^README'
```

## Monitoring and Adjustments

The path patterns may need adjustment over time as the codebase evolves. If you notice:

- False positives (builds running when not needed)
- False negatives (builds not running when needed)

Please open an issue with details so we can refine the patterns.

## Related

- GitHub Issue: [#269](https://github.com/arkavo-org/arkavo-edge/issues/269)
- Workflow file: `.github/workflows/feature.yaml`
- Release workflow: `.github/workflows/release.yaml` (unchanged)