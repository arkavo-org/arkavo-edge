#!/bin/bash

# Launch helper for the Fullstack Web App collaboration demo
# Starts or stops the backend and frontend Arkavo agents

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ARKAVO_BIN="$PROJECT_ROOT/target/debug/arkavo"
PID_FILE="$SCRIPT_DIR/.agent_pids"
LOG_ROOT="$SCRIPT_DIR/logs"

BLUE='\033[0;34m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

print() {
    local color="$1"; shift
    printf "%b%s%b\n" "$color" "$*" "$NC"
}

ensure_binary() {
    if [[ ! -x "$ARKAVO_BIN" ]]; then
        print "$RED" "arkavo binary not found at $ARKAVO_BIN"
        print "$YELLOW" "Run 'cargo build' from the repo root before launching agents."
        exit 1
    fi
}

ensure_dirs() {
    mkdir -p "$LOG_ROOT"
    mkdir -p "$SCRIPT_DIR/backend-agent/logs" "$SCRIPT_DIR/frontend-agent/logs"
}

port_ready() {
    local port="$1"
    if lsof -Pi ":$port" -sTCP:LISTEN >/dev/null 2>&1; then
        return 1
    fi
    return 0
}

start_agent() {
    local name="$1"; shift
    local dir="$1"; shift
    local port="$1"; shift
    local log_file="$LOG_ROOT/${name}.log"

    if ! port_ready "$port"; then
        print "$RED" "Port $port already in use; cannot start $name"
        return 1
    fi

    local pid
    pid=$(
        cd "$dir" && \
        nohup "$ARKAVO_BIN" agent run >"$log_file" 2>&1 & \
        echo $!
    )
    echo "$pid" >> "$PID_FILE"
    print "$BLUE" "Starting $name (PID $pid, port $port)"

    for _ in {1..30}; do
        if curl -sf "http://127.0.0.1:$port/health" >/dev/null 2>&1; then
            print "$GREEN" "$name is healthy"
            return 0
        fi
        sleep 1
    done

    print "$RED" "$name failed to report healthy status within 30s"
    return 1
}

stop_agents() {
    if [[ ! -f "$PID_FILE" ]]; then
        print "$YELLOW" "No PID file found; nothing to stop"
        return 0
    fi

    while read -r pid; do
        if kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
            print "$BLUE" "Stopped process $pid"
        fi
    done < "$PID_FILE"

    rm -f "$PID_FILE"
    pkill -f "arkavo agent run" 2>/dev/null || true
    print "$GREEN" "All agent processes terminated"
}

check_status() {
    local healthy=true

    for tuple in "backend-agent:8351" "frontend-agent:8352"; do
        local name="${tuple%%:*}"
        local port="${tuple##*:}"
        if curl -sf "http://127.0.0.1:$port/health" >/dev/null 2>&1; then
            print "$GREEN" "$name healthy on port $port"
        else
            print "$RED" "$name unavailable on port $port"
            healthy=false
        fi
    done

    $healthy
}

show_logs() {
    print "$BLUE" "Logs live under $LOG_ROOT"
    ls -1 "$LOG_ROOT" 2>/dev/null || true
    print "$YELLOW" "Use 'tail -f $LOG_ROOT/backend-agent.log $LOG_ROOT/frontend-agent.log' for streaming output"
}

main() {
    local cmd="${1:-start}"
    ensure_binary
    ensure_dirs

    case "$cmd" in
        start)
            stop_agents >/dev/null 2>&1 || true
            : >"$PID_FILE"
            start_agent backend-agent "$SCRIPT_DIR/backend-agent" 8351
            start_agent frontend-agent "$SCRIPT_DIR/frontend-agent" 8352
            check_status || exit 1
            show_logs
            ;;
        stop)
            stop_agents
            ;;
        restart)
            stop_agents
            sleep 2
            "$0" start
            ;;
        status)
            check_status
            ;;
        logs)
            show_logs
            ;;
        *)
            print "$YELLOW" "Usage: $0 [start|stop|restart|status|logs]"
            exit 1
            ;;
    esac
}

main "$@"
