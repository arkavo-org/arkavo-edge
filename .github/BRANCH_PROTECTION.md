# Branch Protection Rules

## Required Checks for Main Branch

To ensure code quality and proper versioning, the following GitHub branch protection rules should be configured for the `main` branch:

### Setting Up Branch Protection

1. Go to **Settings** → **Branches** in your GitHub repository
2. Click **Add rule** or edit the existing rule for `main`
3. Enable the following settings:

#### Required Status Checks
Enable **Require status checks to pass before merging** and add these checks:
- ✅ `Lint`
- ✅ `Test`
- ✅ `Build Test (x86_64-unknown-linux-musl, ubuntu-latest)`
- ✅ `Build Test (aarch64-apple-darwin, macos-latest)`
- ✅ `Performance Check`
- ✅ `Smoke Test Linux`
- ✅ `Smoke Test macOS`
- ✅ `Release Readiness`
- ✅ **`Verify Version Increment`** ← This ensures version is updated

#### Additional Recommended Settings
- ✅ **Require branches to be up to date before merging**
- ✅ **Require conversation resolution before merging**
- ✅ **Dismiss stale pull request approvals when new commits are pushed**
- ✅ **Include administrators** (optional, but recommended)

### Version Check Details

The `Verify Version Increment` check ensures that:
1. The version in `Cargo.toml` has been incremented compared to the `main` branch
2. The version follows semantic versioning format (X.Y.Z)
3. The new version is greater than the current version on `main`

This prevents accidental merges without updating the version, which is crucial for proper release management.

### Bypassing Version Check (Emergency Only)

In rare cases where you need to merge without incrementing the version (e.g., documentation-only changes), repository administrators can:
1. Use the "Merge without waiting for requirements to be met" option
2. Or temporarily disable the check in branch protection settings

**Note**: This should be used sparingly and only for changes that truly don't warrant a version bump.