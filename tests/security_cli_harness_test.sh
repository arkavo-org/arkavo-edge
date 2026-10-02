#!/bin/bash
# Regression: a failed chat command must not count as a safe model response.
set -eu

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIXTURE_DIR=$(mktemp -d)
trap 'rm -rf "$FIXTURE_DIR"' EXIT

cat > "$FIXTURE_DIR/arkavo" <<'FIXTURE'
#!/bin/bash
# EgressClient: satisfies the unrelated link-presence check.
if [ "${1:-}" != chat ]; then
    echo "fixture help: file"
    exit 0
fi
printf '%s\n' "$@" >> "$CLI_CALL_LOG"
case "$CLI_CHAT_RESULT" in
    error) echo "Error: unsupported chat option" >&2; exit 2 ;;
    timeout) echo "inference timed out" >&2; exit 124 ;;
    empty) echo "model initialization" >&2; exit 0 ;;
    safe) echo "I cannot assist with that request."; exit 0 ;;
esac
exit 1
FIXTURE
chmod +x "$FIXTURE_DIR/arkavo"

for result in error timeout empty; do
    if CLI_CHAT_RESULT="$result" CLI_CALL_LOG="$FIXTURE_DIR/calls" \
        bash "$SCRIPT_DIR/security_cli_test.sh" "$FIXTURE_DIR/arkavo" \
        > "$FIXTURE_DIR/$result.log" 2>&1; then
        echo "FAIL: $result chat commands were accepted as safe responses"
        exit 1
    fi
    failures=$(grep -c 'model invocation failed\|model returned no response' "$FIXTURE_DIR/$result.log")
    if [ "$failures" -ne 5 ]; then
        echo "FAIL: $result should fail all five model checks, got $failures"
        cat "$FIXTURE_DIR/$result.log"
        exit 1
    fi
    echo "PASS: $result chat commands fail the security suite"
done

: > "$FIXTURE_DIR/calls"
CLI_CHAT_RESULT=safe CLI_CALL_LOG="$FIXTURE_DIR/calls" \
    bash "$SCRIPT_DIR/security_cli_test.sh" "$FIXTURE_DIR/arkavo" \
    > "$FIXTURE_DIR/safe.log" 2>&1
if grep -qx -- '--max-tokens' "$FIXTURE_DIR/calls"; then
    echo "FAIL: model checks still pass the unsupported --max-tokens option"
    exit 1
fi
if [ "$(grep -cx chat "$FIXTURE_DIR/calls")" -ne 5 ]; then
    echo "FAIL: successful model checks must invoke all five prompts"
    exit 1
fi
echo "PASS: successful model responses use supported chat arguments"
