#!/bin/bash

# Scenario runner orchestrating backend and frontend agents via the A2A protocol

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKEND_URL="http://127.0.0.1:8351"
FRONTEND_URL="http://127.0.0.1:8352"

GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

say() {
    local color="$1"; shift
    printf "%b%s%b\n" "$color" "$*" "$NC"
}

rpc_call() {
    local url="$1"
    local method="$2"
    local params="$3"
    local id="${4:-1}"
    curl -s -X POST "$url/rpc" \
        -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"$method\",\"params\":$params,\"id\":$id}" || true
}

require_agents() {
    say "$CYAN" "Checking agent health..."
    for tuple in "backend-agent:$BACKEND_URL" "frontend-agent:$FRONTEND_URL"; do
        local name="${tuple%%:*}"
        local url="${tuple#*:}"
        if curl -sf "$url/health" >/dev/null 2>&1; then
            say "$GREEN" "$name ready at $url"
        else
            say "$RED" "$name not reachable at $url"
            echo "Launch agents first using ./launch_agents.sh" >&2
            exit 1
        fi
    done
}

send_backend_task() {
    local title="$1"
    local body="$2"
    rpc_call "$BACKEND_URL" "message/send" "$(cat <<JSON
{
  "request": {
    "message": {
      "from_agent": "frontend-agent",
      "to_agent": "backend-agent",
      "message_type": "task_assignment",
      "content": "$title",
      "metadata": {
        "summary": "$title",
        "details": "$body",
        "kind": "backend-task"
      }
    }
  }
}
JSON
)" >/dev/null
}

send_frontend_task() {
    local title="$1"
    local body="$2"
    rpc_call "$FRONTEND_URL" "message/send" "$(cat <<JSON
{
  "request": {
    "message": {
      "from_agent": "backend-agent",
      "to_agent": "frontend-agent",
      "message_type": "task_assignment",
      "content": "$title",
      "metadata": {
        "summary": "$title",
        "details": "$body",
        "kind": "frontend-task"
      }
    }
  }
}
JSON
)" >/dev/null
}

request_status() {
    local url="$1"
    local requester="$2"
    local target="$3"
    rpc_call "$url" "agent_query" "$(cat <<JSON
{
  "request": {
    "from_agent_id": "$requester",
    "to_agent_id": "$target",
    "query": "What is your current delivery status for the web app collaboration?"
  }
}
JSON
)"
}

scenario_bootstrap() {
    say "$CYAN" "Kickstarting fullstack delivery..."
    send_backend_task "Implement launch management API" \
        "Follow project/backend/README.md deliverables, ensure OpenAPI published and migrations committed."
    send_frontend_task "Scaffold dashboard UI" \
        "Create responsive dashboard, hook filters, wire form validation to backend contract per project/frontend/README.md."
    say "$GREEN" "Bootstrap tasks dispatched"
}

scenario_bugfix() {
    say "$CYAN" "Assigning regression fixes..."
    send_backend_task "Fix validation and update behavior" \
        "Resolve HTTP 200 on validation failure, return 422 with error payload, persist deployment links on PUT."
    send_frontend_task "Resolve edit regressions" \
        "Prevent past dates for completed launches and preserve deployment link edits with diffing strategy."
    say "$GREEN" "Bug fix work queued"
}

scenario_enhance() {
    say "$CYAN" "Triggering enhancement sprint..."
    send_backend_task "Add metrics and archive pipeline" \
        "Implement metrics endpoint, archive job with report, and SSE stream for live updates."
    send_frontend_task "Deliver metrics UI and notifications" \
        "Render health summary panel, wire archive alerts, and surface toast success/error flows."
    say "$GREEN" "Enhancement directives sent"
}

scenario_status() {
    say "$CYAN" "Requesting status from both agents..."
    say "$YELLOW" "Backend response:" && request_status "$BACKEND_URL" "frontend-agent" "backend-agent"
    echo
    say "$YELLOW" "Frontend response:" && request_status "$FRONTEND_URL" "backend-agent" "frontend-agent"
    echo
}

scenario_regression_verification() {
    say "$CYAN" "Requesting regression verification hooks..."
    rpc_call "$BACKEND_URL" "message/send" "$(cat <<'JSON'
{
  "request": {
    "message": {
      "from_agent": "monitor-bot",
      "to_agent": "backend-agent",
      "message_type": "verification_request",
      "content": "Run cargo test -p launch-backend and confirm regression coverage was updated.",
      "metadata": {
        "kind": "verification",
        "checks": [
          "cargo test -p launch-backend",
          "cargo clippy -- -D warnings"
        ]
      }
    }
  }
}
JSON
)" >/dev/null

    rpc_call "$FRONTEND_URL" "message/send" "$(cat <<'JSON'
{
  "request": {
    "message": {
      "from_agent": "monitor-bot",
      "to_agent": "frontend-agent",
      "message_type": "verification_request",
      "content": "Run npm test, npm run lint, and publish a summary via A2A when complete.",
      "metadata": {
        "kind": "verification",
        "checks": [
          "npm test",
          "npm run lint",
          "npm run typecheck"
        ]
      }
    }
  }
}
JSON
)" >/dev/null
    say "$GREEN" "Verification commands dispatched"
}

main() {
    local scenario="${1:-all}"
    require_agents

    case "$scenario" in
        bootstrap)
            scenario_bootstrap
            ;;
        bugfix)
            scenario_bugfix
            ;;
        enhance)
            scenario_enhance
            ;;
        status)
            scenario_status
            ;;
        verify)
            scenario_regression_verification
            ;;
        all)
            scenario_bootstrap
            sleep 2
            scenario_bugfix
            sleep 2
            scenario_enhance
            sleep 2
            scenario_status
            ;;
        *)
            say "$YELLOW" "Usage: $0 [bootstrap|bugfix|enhance|status|verify|all]"
            exit 1
            ;;
    esac
}

main "$@"
