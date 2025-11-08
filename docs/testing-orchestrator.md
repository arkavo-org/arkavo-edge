# GitHub Orchestrator Manual Testing Guide

This guide provides step-by-step instructions for manually testing the GitHub orchestrator functionality in PR #328.

## Prerequisites

### Required Setup

- Rust toolchain installed (`cargo --version` should work)
- GitHub account with a test repository
- GitHub Personal Access Token (PAT) with repo permissions
- For webhook mode: GitHub App credentials

### Build the Project

```bash
cd /path/to/arkavo-edge
cargo build --package arkavo-cli
```

The binary will be at `./target/debug/arkavo`

## Test Scenarios

### Scenario 1: Polling Mode - Single Issue Processing

Tests the ability to process a specific GitHub issue on demand.

**Setup:**
1. Create a test issue in your GitHub repository
2. Note the issue number (e.g., #42)
3. Set your GitHub token:
   ```bash
   export GITHUB_TOKEN=ghp_your_token_here
   ```

**Test Steps:**
```bash
./target/debug/arkavo orchestrator poll \
  --repo owner/repo \
  --issue 42
```

**Expected Results:**
- Log message: "Processing issue #42 from owner/repo"
- Log message: "Issue #42: [issue title]"
- Orchestrator initializes successfully
- Issue processing completes without errors
- Log message: "Issue processing complete"

**Failure Indicators:**
- Error about missing token
- GitHub API authentication errors (401)
- Repository not found errors (404)
- Orchestrator initialization failures

### Scenario 2: Polling Mode - One-Shot Scan

Tests fetching all new issues since last poll in a single run.

**Setup:**
1. Create 2-3 test issues in your repository
2. Set GitHub token as above

**Test Steps:**
```bash
./target/debug/arkavo orchestrator poll \
  --repo owner/repo \
  --once
```

**Expected Results:**
- Log message: "Starting GitHub polling for repository: owner/repo"
- Log message: "Running in one-shot mode"
- Log message: "Found N issues to process"
- Each issue gets processed: "Processing issue #N: [title]"
- State file created at `~/.arkavo/poll-state-owner-repo.json`
- Log message: "One-shot mode complete"

**Verify State Persistence:**
```bash
cat ~/.arkavo/poll-state-owner-repo.json
```

Should show:
```json
{
  "last_poll": "2025-11-07T...",
  "processed_issues": {
    "42": "2025-11-07T...",
    "43": "2025-11-07T..."
  },
  "max_processed_issues": 1000
}
```

**Failure Indicators:**
- No issues found when they should exist
- Same issue processed multiple times
- State file not created
- Errors parsing repository name

### Scenario 3: Polling Mode - Continuous Polling

Tests the continuous polling loop with custom interval.

**Setup:**
1. Create a test repository with at least one issue
2. Set GitHub token

**Test Steps:**
```bash
./target/debug/arkavo orchestrator poll \
  --repo owner/repo \
  --interval 30
```

**Expected Results:**
- Log message: "Polling every 30 seconds"
- Continuous polling loop starts
- Log message: "Polling owner/repo for new issues" every 30 seconds
- Processes any new issues found
- State file updates after each poll cycle

**Manual Actions During Test:**
1. Let it poll 2-3 times (observe timestamps)
2. Create a new issue in the repository
3. Wait for next poll cycle
4. Verify new issue is detected and processed
5. Create another issue
6. Verify it's only processed once (state tracking works)
7. Press Ctrl+C to stop

**Failure Indicators:**
- Polling stops unexpectedly
- Same issue processed multiple times
- New issues not detected
- Interval not respected (timing is off)

### Scenario 4: Label Filtering

Tests filtering issues by labels during polling.

**Setup:**
1. Create issues with different labels:
   - Issue #1 with label "bug"
   - Issue #2 with label "enhancement"
   - Issue #3 with no labels
2. Set GitHub token

**Test Steps:**
```bash
./target/debug/arkavo orchestrator poll \
  --repo owner/repo \
  --labels bug \
  --once
```

**Expected Results:**
- Only issue #1 (with "bug" label) is processed
- Issues #2 and #3 are ignored
- Log shows: "Found 1 issues to process"

**Additional Test - Multiple Labels:**
```bash
./target/debug/arkavo orchestrator poll \
  --repo owner/repo \
  --labels "bug,enhancement" \
  --once
```

Should process both #1 and #2, but not #3.

### Scenario 5: Rate Limiting Behavior

Tests retry logic when hitting GitHub API rate limits.

**Note:** This is difficult to test manually without actually hitting rate limits. Best tested by:

1. Using a token with very low rate limit remaining
2. Or reviewing the code logic in `github_api.rs:137-165`

**Setup:**
1. Check current rate limit status:
   ```bash
   curl -H "Authorization: Bearer $GITHUB_TOKEN" \
     https://api.github.com/rate_limit
   ```

**If rate limit is low (< 10 remaining):**
```bash
./target/debug/arkavo orchestrator poll \
  --repo owner/repo \
  --once
```

**Expected Results (if rate limited):**
- Log warning: "Rate limited on 'owner/repo'. Waiting X seconds (attempt 1/3)"
- Automatic retry after wait period
- Up to 3 retry attempts
- Error after 3 failed attempts: "Exceeded maximum retry attempts"

### Scenario 6: Error Handling - Invalid Repository

Tests error handling for malformed repository names.

**Test Steps:**

```bash
# Missing owner
./target/debug/arkavo orchestrator poll --repo just-repo --once

# Missing repo name
./target/debug/arkavo orchestrator poll --repo owner/ --once

# Empty parts
./target/debug/arkavo orchestrator poll --repo "  /  " --once

# Too many slashes
./target/debug/arkavo orchestrator poll --repo owner/repo/extra --once
```

**Expected Results:**
- Clear error message for each case
- Error: "Invalid repository format. Expected 'owner/repo', got '...'"
- Or: "Owner and repository name cannot be empty"
- Process exits with error code

### Scenario 7: Error Handling - Missing Token

Tests error messages when GitHub token is not provided.

**Test Steps:**
```bash
unset GITHUB_TOKEN
./target/debug/arkavo orchestrator poll --repo owner/repo --once
```

**Expected Results:**
- Error: "GitHub token required for 'owner/repo'. Set GITHUB_TOKEN environment variable or use --token"
- Process exits cleanly

### Scenario 8: Error Handling - Invalid Token

Tests behavior with invalid authentication.

**Test Steps:**
```bash
export GITHUB_TOKEN=invalid_token_123
./target/debug/arkavo orchestrator poll --repo owner/repo --once
```

**Expected Results:**
- GitHub API error (401 Unauthorized)
- Clear error message about authentication failure
- Process exits with error code

### Scenario 9: Error Handling - Repository Not Found

Tests behavior when repository doesn't exist.

**Test Steps:**
```bash
./target/debug/arkavo orchestrator poll \
  --repo nonexistent-owner/nonexistent-repo \
  --once
```

**Expected Results:**
- GitHub API error (404 Not Found)
- Error message: "GitHub API error for repository nonexistent-owner/nonexistent-repo: 404"
- Process exits with error code

### Scenario 10: State Cleanup (1000 Issue Limit)

Tests automatic cleanup when processed issues exceed limit.

**Setup:**
This requires a repository with many issues or manual state file manipulation.

**Manual State File Test:**
1. Create state file with 1005 issues:
   ```bash
   mkdir -p ~/.arkavo
   # Use a script to generate JSON with 1005 issues
   # Each with different timestamps
   ```

2. Run polling:
   ```bash
   ./target/debug/arkavo orchestrator poll --repo owner/repo --once
   ```

3. Check state file:
   ```bash
   cat ~/.arkavo/poll-state-owner-repo.json | jq '.processed_issues | length'
   ```

**Expected Results:**
- State file contains exactly 1000 issues (MAX_PROCESSED_ISSUES)
- Oldest 5 issues (by timestamp) are removed
- Newest 1000 issues are retained

### Scenario 11: Webhook Mode (Requires GitHub App)

Tests webhook server for receiving GitHub events.

**Setup:**
1. Create a GitHub App with webhook permissions
2. Set webhook URL to your server (use ngrok for local testing)
3. Set environment variables:
   ```bash
   export GITHUB_WEBHOOK_SECRET=your_webhook_secret
   export GITHUB_APP_ID=123456
   export GITHUB_APP_PRIVATE_KEY_PATH=/path/to/private-key.pem
   ```

**Test Steps:**
```bash
./target/debug/arkavo orchestrator start --port 3000
```

**Expected Results:**
- Log: "Starting GitHub orchestrator server"
- Log: "Webhook server will listen on port 3000"
- Log: "GitHub App ID: 123456"
- Log: "Webhook secret: ****..." (masked)
- Log: "Webhook server listening on 0.0.0.0:3000"
- Log: "Orchestrator is ready to process GitHub events"

**Manual Trigger:**
1. Create an issue in a repository with the GitHub App installed
2. Check server logs for:
   - "Received issue event"
   - Issue number and repository name
   - "Failed to handle issue event" or successful processing

**Failure Indicators:**
- Port already in use errors
- Webhook signature verification failures
- GitHub App authentication errors

### Scenario 12: HTTP Client Timeout

Tests request timeout handling.

**Setup:**
This is difficult to test without a slow/hanging GitHub API. Best verified by:

1. Reviewing code in `github_api.rs:28-34` for timeout configuration
2. Checking that GITHUB_API_TIMEOUT_SECS = 30 is used

**Code Review Verification:**
```bash
grep -n "GITHUB_API_TIMEOUT_SECS" crates/arkavo-cli/src/commands/orchestrator/github_api.rs
```

Should show timeout is set in `create_client()` function.

### Scenario 13: User-Agent Header

Tests that proper User-Agent is sent to GitHub API.

**Verification Method:**

Check code in `constants.rs`:
```bash
cat crates/arkavo-cli/src/commands/orchestrator/constants.rs | grep USER_AGENT
```

Should show: `pub(super) const USER_AGENT: &str = concat!("arkavo-edge/", env!("CARGO_PKG_VERSION"));`

**Runtime Verification:**
Use a proxy or GitHub API logs to verify the User-Agent header includes version number like:
`arkavo-edge/0.38.3`

### Scenario 14: Real GitHub Data Integration

Verifies that real GitHub API data is used (not placeholders).

**Test Steps:**
1. Process a real GitHub issue with specific attributes:
   - Custom labels
   - Specific user
   - Body content with markdown
   - Created/updated timestamps

```bash
./target/debug/arkavo orchestrator poll \
  --repo owner/repo \
  --issue 42
```

2. Add debug logging to verify data:
   - Issue ID is not 0
   - Repository ID is not 0
   - User avatar_url is not empty
   - Default branch matches actual repo default branch (not hardcoded "main")
   - Timestamps are real ISO 8601 dates

**Expected Results:**
- All fields populated with real data from GitHub API
- No placeholder values (0, empty strings, "main" for non-main repos)
- Timestamps match GitHub issue creation times

## Cleanup After Testing

```bash
# Remove state files
rm ~/.arkavo/poll-state-*.json

# Remove task database
rm ~/.arkavo/orchestrator-tasks.db

# Unset environment variables
unset GITHUB_TOKEN
unset GITHUB_WEBHOOK_SECRET
unset GITHUB_APP_ID
unset GITHUB_APP_PRIVATE_KEY_PATH
```

## Common Issues and Troubleshooting

### "Failed to create HTTP client"
- Check network connectivity
- Verify no proxy issues
- Check Rust/OpenSSL installation

### "Rate limited" errors appearing immediately
- Check rate limit status: `curl -H "Authorization: Bearer $GITHUB_TOKEN" https://api.github.com/rate_limit`
- Wait for rate limit reset or use different token

### State file not created
- Check `~/.arkavo/` directory exists and is writable
- Verify permissions: `ls -la ~/.arkavo/`

### Webhook signature verification fails
- Verify GITHUB_WEBHOOK_SECRET matches GitHub App configuration
- Check webhook payload is valid JSON
- Verify webhook content-type is application/json

### Orchestrator initialization fails
- Check all required dependencies are available
- Verify database path is writable
- Check MCP client can initialize

## Performance Benchmarks

While testing, observe:

- **Issue processing time**: Should complete in < 5 seconds per issue
- **API request latency**: Most requests < 1 second (network dependent)
- **State file operations**: < 100ms for read/write
- **Memory usage**: Should remain stable during continuous polling
- **Poll accuracy**: Interval should be accurate within ±1 second

## Reporting Issues

When reporting bugs found during testing, include:

1. Exact command used
2. Full error message or unexpected behavior
3. Environment details (OS, Rust version)
4. Repository used for testing (if public)
5. Relevant log output (use `RUST_LOG=debug` for verbose logs)

Example:
```bash
RUST_LOG=debug ./target/debug/arkavo orchestrator poll --repo owner/repo --once 2>&1 | tee test-output.log
```
