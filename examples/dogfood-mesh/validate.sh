#!/bin/bash
# Mechanical validation for dogfood mesh agent output.
# Extracts code from agent response, applies to target file,
# runs cargo fmt/clippy/test, commits on pass, reverts on fail.
#
# Usage: ./validate.sh <crate-name> <response-file> <agent-type>
# agent-type: code-reviewer | test-writer
#
# Exit codes:
#   0 = validation passed, changes committed
#   1 = validation failed, changes reverted
#   2 = no actionable output found

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
LOG_DIR="$SCRIPT_DIR/logs"

CRATE="${1:?Usage: $0 <crate-name> <response-file> <agent-type>}"
RESPONSE_FILE="${2:?Usage: $0 <crate-name> <response-file> <agent-type>}"
AGENT_TYPE="${3:?Usage: $0 <crate-name> <response-file> <agent-type>}"

mkdir -p "$LOG_DIR"

log() {
    echo "[validate] $(date +%H:%M:%S) $*" | tee -a "$LOG_DIR/validate.log"
}

# Revert all unstaged changes in the project
revert_changes() {
    cd "$PROJECT_ROOT"
    git checkout -- . 2>/dev/null || true
    git clean -fd -- "crates/$CRATE" 2>/dev/null || true
}

# Extract fenced code blocks from agent response
extract_code_blocks() {
    local file="$1"
    awk '
        /^```rust/ { in_block=1; next }
        /^```/ { if (in_block) { in_block=0; print "---BLOCK_SEPARATOR---" }; next }
        in_block { print }
    ' "$file"
}

# Find the target file and insertion point from agent response
# Looks for FILE: directives in the response
extract_file_directive() {
    local file="$1"
    grep -oP '(?<=FILE:\s).*' "$file" | head -1 | tr -d '[:space:]'
}

validate_test_writer() {
    local response="$1"

    # Extract target file path from response
    local target_file
    target_file=$(extract_file_directive "$response")

    if [ -z "$target_file" ]; then
        log "SKIP: No FILE: directive found in test-writer response"
        return 2
    fi

    local full_path="$PROJECT_ROOT/$target_file"
    if [ ! -f "$full_path" ]; then
        log "FAIL: Target file not found: $target_file"
        return 1
    fi

    # Extract test code blocks
    local test_code
    test_code=$(extract_code_blocks "$response")

    if [ -z "$test_code" ]; then
        log "SKIP: No rust code blocks found in response"
        return 2
    fi

    # Remove block separators for insertion
    test_code=$(echo "$test_code" | grep -v '^---BLOCK_SEPARATOR---$')

    # Check if file already has a #[cfg(test)] module
    if grep -q '#\[cfg(test)\]' "$full_path"; then
        # Find the last closing brace of the test module and insert before it
        # Strategy: find line number of last } in file, insert before it
        local last_brace_line
        last_brace_line=$(grep -n '^}' "$full_path" | tail -1 | cut -d: -f1)

        if [ -n "$last_brace_line" ]; then
            # Insert test code before the last closing brace
            local tmp_file
            tmp_file=$(mktemp)
            head -n $((last_brace_line - 1)) "$full_path" > "$tmp_file"
            echo "" >> "$tmp_file"
            echo "$test_code" >> "$tmp_file"
            echo "" >> "$tmp_file"
            tail -n +"$last_brace_line" "$full_path" >> "$tmp_file"
            cp "$tmp_file" "$full_path"
            rm -f "$tmp_file"
        fi
    else
        # Append a new test module
        cat >> "$full_path" <<EOF

#[cfg(test)]
mod tests {
    use super::*;

$test_code
}
EOF
    fi

    log "Applied test code to $target_file"

    # Run validation pipeline
    run_cargo_checks "$target_file"
}

validate_code_reviewer() {
    # Code reviewer output is informational (JSON report).
    # Validation: check that it's valid JSON and contains expected fields.
    local response="$1"

    # Try to extract JSON from response (may be in a code block)
    local json_content
    json_content=$(awk '
        /^```json/ { in_block=1; next }
        /^```/ { if (in_block) { in_block=0 }; next }
        in_block { print }
    ' "$response")

    if [ -z "$json_content" ]; then
        # Try the whole response as JSON
        json_content=$(cat "$response")
    fi

    # Validate JSON structure
    if echo "$json_content" | jq -e 'if type == "array" then length > 0 else .file_path != null end' > /dev/null 2>&1; then
        local item_count
        item_count=$(echo "$json_content" | jq 'if type == "array" then length else 1 end' 2>/dev/null || echo "0")
        log "PASS: Code review report contains $item_count findings"
        return 0
    else
        log "FAIL: Code review output is not valid structured JSON"
        return 1
    fi
}

run_cargo_checks() {
    local changed_file="$1"

    cd "$PROJECT_ROOT"

    # Step 1: cargo fmt check
    log "Running cargo fmt --check..."
    if ! cargo fmt -- --check 2>>"$LOG_DIR/validate.log"; then
        log "FAIL: cargo fmt check failed, auto-formatting..."
        cargo fmt 2>>"$LOG_DIR/validate.log"
    fi

    # Step 2: cargo check (fast compilation test)
    log "Running cargo check -p $CRATE..."
    if ! cargo check -p "$CRATE" 2>>"$LOG_DIR/validate.log"; then
        log "FAIL: cargo check failed"
        revert_changes
        return 1
    fi

    # Step 3: cargo clippy
    log "Running cargo clippy -p $CRATE..."
    if ! cargo clippy -p "$CRATE" -- -D warnings 2>>"$LOG_DIR/validate.log"; then
        log "FAIL: cargo clippy failed"
        revert_changes
        return 1
    fi

    # Step 4: cargo test
    log "Running cargo test -p $CRATE..."
    if ! cargo test -p "$CRATE" 2>>"$LOG_DIR/validate.log"; then
        log "FAIL: cargo test failed"
        revert_changes
        return 1
    fi

    # All checks passed — commit
    log "PASS: All cargo checks passed"
    git add "$changed_file"
    git commit -m "[dogfood] Add tests for $CRATE ($(basename "$changed_file"))"
    log "Committed changes to $changed_file"
    return 0
}

# --- Main ---

if [ ! -f "$RESPONSE_FILE" ]; then
    log "Error: response file not found: $RESPONSE_FILE"
    exit 1
fi

# Check response is non-empty and substantive
response_length=$(wc -c < "$RESPONSE_FILE")
if [ "$response_length" -lt 50 ]; then
    log "SKIP: Response too short ($response_length bytes)"
    exit 2
fi

case "$AGENT_TYPE" in
    test-writer)
        validate_test_writer "$RESPONSE_FILE"
        ;;
    code-reviewer)
        validate_code_reviewer "$RESPONSE_FILE"
        ;;
    *)
        log "Error: unknown agent type: $AGENT_TYPE"
        exit 1
        ;;
esac
