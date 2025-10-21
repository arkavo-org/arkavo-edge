#!/bin/bash

# Lightweight monitor that runs local quality checks and reports results via A2A messages

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXAMPLE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BACKEND_URL="http://127.0.0.1:8351"
FRONTEND_URL="http://127.0.0.1:8352"

CYAN='\033[0;36m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

say() {
    local color="$1"; shift
    printf "%b%s%b\n" "$color" "$*" "$NC"
}

require_tool() {
    local tool="$1"
    if ! command -v "$tool" >/dev/null 2>&1; then
        say "$RED" "Required tool '$tool' not found in PATH"
        return 1
    fi
    return 0
}

rpc_call() {
    local url="$1"
    local method="$2"
    local params="$3"
    curl -s -X POST "$url/rpc" \
        -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"$method\",\"params\":$params,\"id\":99}" >/dev/null || true
}

json_escape() {
    python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))'
}

notify_agent() {
    local url="$1"
    local to_agent="$2"
    local summary="$3"
    local severity="$4"
    local details="$5"
    local summary_json details_json
    summary_json=$(json_escape <<<"$summary")
    details_json=$(json_escape <<<"$details")
    rpc_call "$url" "message/send" "$(cat <<JSON
{
  "request": {
    "message": {
      "from_agent": "monitor-bot",
      "to_agent": "$to_agent",
      "message_type": "monitor_report",
      "content": $summary_json,
      "metadata": {
        "severity": "$severity",
        "details": $details_json
      }
    }
  }
}
JSON
)"
}

check_backend() {
    local backend_dir="$EXAMPLE_ROOT/project/backend"
    if [[ ! -f "$backend_dir/Cargo.toml" ]]; then
        say "$YELLOW" "Backend workspace not initialized; skipping backend checks"
        return 0
    fi

    if ! require_tool cargo; then
        notify_agent "$BACKEND_URL" "backend-agent" "Backend checks skipped: cargo missing" "error" "Install Rust toolchain to enable automated verification."
        return 0
    fi

    say "$CYAN" "Running backend checks..."
    local output status=0
    output=$(cd "$backend_dir" && cargo test -p launch-backend 2>&1) || status=$?
    output+=$'\n'
    local clippy_status=0
    output+=$(cd "$backend_dir" && cargo clippy -p launch-backend -- -D warnings 2>&1) || clippy_status=$?

    if [[ $status -eq 0 && $clippy_status -eq 0 ]]; then
        say "$GREEN" "Backend checks passed"
        notify_agent "$BACKEND_URL" "backend-agent" "Backend checks passed" "ok" "$output"
    else
        say "$RED" "Backend checks failed"
        notify_agent "$BACKEND_URL" "backend-agent" "Backend checks failed" "error" "$output"
    fi
}

check_frontend() {
    local frontend_dir="$EXAMPLE_ROOT/project/frontend"
    if [[ ! -f "$frontend_dir/package.json" ]]; then
        say "$YELLOW" "Frontend workspace not initialized; skipping frontend checks"
        return 0
    fi

    if ! require_tool npm; then
        notify_agent "$FRONTEND_URL" "frontend-agent" "Frontend checks skipped: npm missing" "error" "Install Node.js and npm to enable automated verification."
        return 0
    fi

    say "$CYAN" "Running frontend checks..."
    local output status=0
    output=$(cd "$frontend_dir" && npm test -- --watch=false 2>&1) || status=$?
    output+=$'\n'
    local lint_status=0
    output+=$(cd "$frontend_dir" && npm run lint 2>&1) || lint_status=$?
    output+=$'\n'
    local type_status=0
    output+=$(cd "$frontend_dir" && npm run typecheck 2>&1) || type_status=$?

    if [[ $status -eq 0 && $lint_status -eq 0 && $type_status -eq 0 ]]; then
        say "$GREEN" "Frontend checks passed"
        notify_agent "$FRONTEND_URL" "frontend-agent" "Frontend checks passed" "ok" "$output"
    else
        say "$RED" "Frontend checks failed"
        notify_agent "$FRONTEND_URL" "frontend-agent" "Frontend checks failed" "error" "$output"
    fi
}

request_progress() {
    say "$CYAN" "Collecting live status from agents"
    local backend_resp frontend_resp
    backend_resp=$(rpc_call "$BACKEND_URL" "agent_query" "$(cat <<'JSON'
{
  "request": {
    "from_agent_id": "monitor-bot",
    "to_agent_id": "backend-agent",
    "query": "Provide current status, outstanding bugs, and latest release tag."
  }
}
JSON
)")
    frontend_resp=$(rpc_call "$FRONTEND_URL" "agent_query" "$(cat <<'JSON'
{
  "request": {
    "from_agent_id": "monitor-bot",
    "to_agent_id": "frontend-agent",
    "query": "Summarize shipped UI work, pending bug fixes, and any blockers."
  }
}
JSON
)")
    printf "Backend response: %s\n\n" "$backend_resp"
    printf "Frontend response: %s\n" "$frontend_resp"
}

main() {
    local cmd="${1:-status}"
    case "$cmd" in
        status)
            request_progress
            ;;
        verify)
            check_backend
            check_frontend
            ;;
        backend)
            check_backend
            ;;
        frontend)
            check_frontend
            ;;
        *)
            say "$YELLOW" "Usage: $0 [status|verify|backend|frontend]"
            exit 1
            ;;
    esac
}

main "$@"
