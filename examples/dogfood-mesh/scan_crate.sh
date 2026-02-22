#!/bin/bash
# Scan a crate for quality issues, test gaps, and public API surface.
# Outputs structured JSON for consumption by dogfood mesh agents.
#
# Usage: ./scan_crate.sh <crate-name>
# Example: ./scan_crate.sh arkavo-validation

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

CRATE="${1:?Usage: $0 <crate-name>}"
CRATE_DIR="$PROJECT_ROOT/crates/$CRATE"

if [ ! -d "$CRATE_DIR" ]; then
    echo "Error: crate directory not found: $CRATE_DIR" >&2
    exit 1
fi

# Temporary files for capturing output
CLIPPY_OUT=$(mktemp)
TEST_LIST_OUT=$(mktemp)
PUB_FN_OUT=$(mktemp)
trap 'rm -f "$CLIPPY_OUT" "$TEST_LIST_OUT" "$PUB_FN_OUT"' EXIT

# --- Clippy warnings ---
# Run clippy and capture warnings in JSON format
cargo clippy -p "$CRATE" --message-format=json -- -D warnings 2>/dev/null \
    | jq -c 'select(.reason == "compiler-message") | select(.message.level == "warning") | {
        file: (.message.spans[0].file_name // "unknown"),
        line: (.message.spans[0].line_start // 0),
        message: .message.message,
        severity: "warning",
        code: (.message.code.code // null)
    }' > "$CLIPPY_OUT" 2>/dev/null || true

# --- Test list ---
# List existing test functions
cargo test -p "$CRATE" -- --list 2>/dev/null \
    | grep ': test$' \
    | sed 's/: test$//' \
    > "$TEST_LIST_OUT" 2>/dev/null || true

# --- Public functions ---
# Find all pub fn declarations with file paths and line numbers
find "$CRATE_DIR/src" -name '*.rs' -print0 2>/dev/null | while IFS= read -r -d '' file; do
    # Skip test modules
    in_test=0
    line_num=0
    while IFS= read -r line; do
        line_num=$((line_num + 1))
        # Track test module boundaries (simple heuristic)
        if echo "$line" | grep -q '#\[cfg(test)\]'; then
            in_test=1
        fi
        if [ "$in_test" -eq 0 ]; then
            if echo "$line" | grep -qE '^\s*pub\s+(async\s+)?fn\s+'; then
                fn_name=$(echo "$line" | sed -E 's/.*pub[[:space:]]+(async[[:space:]]+)?fn[[:space:]]+([a-zA-Z_][a-zA-Z0-9_]*).*/\2/')
                rel_path="${file#$PROJECT_ROOT/}"
                printf '%s\t%s\t%d\n' "$rel_path" "$fn_name" "$line_num"
            fi
        fi
    done < "$file"
done > "$PUB_FN_OUT" 2>/dev/null || true

# --- Assemble JSON output ---
echo "{"
echo "  \"crate\": \"$CRATE\","

# Clippy warnings array
echo -n "  \"clippy_warnings\": ["
if [ -s "$CLIPPY_OUT" ]; then
    first=1
    while IFS= read -r warning; do
        [ "$first" -eq 1 ] && first=0 || echo -n ","
        echo -n "$warning"
    done < "$CLIPPY_OUT"
fi
echo "],"

# Test list array
echo -n "  \"test_list\": ["
if [ -s "$TEST_LIST_OUT" ]; then
    first=1
    while IFS= read -r test_name; do
        [ "$first" -eq 1 ] && first=0 || echo -n ","
        echo -n "\"$test_name\""
    done < "$TEST_LIST_OUT"
fi
echo "],"

# Public functions array
echo -n "  \"public_functions\": ["
if [ -s "$PUB_FN_OUT" ]; then
    first=1
    while IFS=$'\t' read -r file fn_name line_num; do
        [ "$first" -eq 1 ] && first=0 || echo -n ","
        echo -n "{\"file\":\"$file\",\"fn\":\"$fn_name\",\"line\":$line_num}"
    done < "$PUB_FN_OUT"
fi
echo "]"

echo "}"
