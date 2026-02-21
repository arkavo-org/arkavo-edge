#!/bin/bash
# Dogfood Learning Mesh — Arkavo Edge Self-Improvement
#
# Unattended overnight run: starts agents, scans each crate, dispatches
# to mesh, validates output, commits passing changes, creates PR.
#
# Usage:
#   ./launch.sh                    # Full run (all safe crates)
#   ./launch.sh arkavo-validation  # Single crate
#   ./launch.sh stop               # Stop agents

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BINARY="${BINARY:-${PROJECT_ROOT}/target/debug/arkavo}"
[ -f "$BINARY" ] || BINARY="${PROJECT_ROOT}/target/release/arkavo"
LOG_DIR="$SCRIPT_DIR/logs"
PID_FILE="$SCRIPT_DIR/.agent_pids"
RESPONSE_DIR="$SCRIPT_DIR/responses"
LOGFILE="$LOG_DIR/dogfood.log"
BRANCH="dogfood/$(date +%Y-%m-%d)"

CRATES=(
    arkavo-test-macros
    arkavo-validation
    arkavo-config-encryption
    arkavo-events
    arkavo-bench
    arkavo-sbe
    arkavo-observability
    arkavo-budget
)

mkdir -p "$LOG_DIR" "$RESPONSE_DIR"

log() { echo "$(date +%Y-%m-%dT%H:%M:%S) $*" >> "$LOGFILE"; }

die() { log "FATAL: $*"; echo "FATAL: $*" >&2; exit 1; }

stop_agents() {
    if [ -f "$PID_FILE" ]; then
        while IFS= read -r pid; do
            kill "$pid" 2>/dev/null || true
        done < "$PID_FILE"
        rm -f "$PID_FILE"
    fi
    pkill -f "arkavo agent" 2>/dev/null || true
    log "Agents stopped"
}

start_agents() {
    stop_agents
    > "$PID_FILE"

    for agent_dir in orchestrator code-reviewer test-writer; do
        local dir="$SCRIPT_DIR/$agent_dir"
        [ -f "$dir/AGENTS.md" ] || continue
        cd "$dir"
        RUST_LOG=info nohup "$BINARY" agent run >> "$LOG_DIR/${agent_dir}.log" 2>&1 &
        echo "$!" >> "$PID_FILE"
        cd "$SCRIPT_DIR"
        log "Started $agent_dir (pid $!)"
    done

    sleep 5
    log "Agents initialized"
}

wait_for_agents() {
    local retries=10
    while [ "$retries" -gt 0 ]; do
        if curl -s http://localhost:8421/.well-known/agent.json >/dev/null 2>&1; then
            return 0
        fi
        sleep 2
        retries=$((retries - 1))
    done
    die "Agents failed to start"
}

scan_and_dispatch() {
    local crate=$1
    local ts
    ts=$(date +%s)
    local scan_file="$RESPONSE_DIR/scan_${crate}_${ts}.json"
    local response_file="$RESPONSE_DIR/response_${crate}_${ts}.txt"

    log "Scanning $crate"
    if ! "$SCRIPT_DIR/scan_crate.sh" "$crate" > "$scan_file" 2>>"$LOGFILE"; then
        log "Scan failed for $crate"
        return 1
    fi

    local scan_content
    scan_content=$(cat "$scan_file")
    log "Scan complete: $(jq -c '{w:.clippy_warnings|length,f:.public_functions|length,t:.test_list|length}' "$scan_file" 2>/dev/null)"

    log "Dispatching $crate to mesh"
    if "$BINARY" task "Analyze the $crate crate in the Arkavo Edge project.

Scan results:
$scan_content

Based on these findings, identify the most impactful test gaps and produce
unit tests that improve coverage for the functions listed above.
Focus on edge cases, error paths, and boundary conditions." \
        --mesh-only --yes --no-validate \
        > "$response_file" 2>>"$LOGFILE"; then
        log "Response: $(wc -c < "$response_file" | tr -d ' ') bytes"
    else
        log "Dispatch failed for $crate"
        return 1
    fi

    # Validate and commit
    if [ -s "$response_file" ]; then
        local rc=0
        "$SCRIPT_DIR/validate.sh" "$crate" "$response_file" "test-writer" >> "$LOGFILE" 2>&1 || rc=$?
        case $rc in
            0) log "PASS: $crate — committed" ;;
            1) log "FAIL: $crate — reverted" ;;
            2) log "SKIP: $crate — no actionable output" ;;
        esac
        return $rc
    else
        log "SKIP: $crate — empty response"
        return 2
    fi
}

run() {
    local targets=("$@")
    [ ${#targets[@]} -gt 0 ] || targets=("${CRATES[@]}")

    [ -f "$BINARY" ] || die "Binary not found: $BINARY"
    command -v jq &>/dev/null || die "jq not found"

    log "=== Dogfood run started: ${targets[*]} ==="

    # Branch
    cd "$PROJECT_ROOT"
    if ! git rev-parse --verify "$BRANCH" >/dev/null 2>&1; then
        git checkout -b "$BRANCH" >> "$LOGFILE" 2>&1
    else
        git checkout "$BRANCH" >> "$LOGFILE" 2>&1
    fi

    # Agents
    start_agents
    wait_for_agents

    # Dispatch each crate
    local pass=0 fail=0 skip=0
    for crate in "${targets[@]}"; do
        local rc=0
        scan_and_dispatch "$crate" || rc=$?
        case $rc in
            0) pass=$((pass + 1)) ;;
            1) fail=$((fail + 1)) ;;
            *) skip=$((skip + 1)) ;;
        esac
    done

    log "=== Results: $pass pass, $fail fail, $skip skip ==="

    # PR if there are commits
    local commits
    commits=$(git log --oneline "$BRANCH" --not main 2>/dev/null | wc -l | tr -d ' ')
    if [ "$commits" -gt 0 ]; then
        log "Creating PR ($commits commits)"
        git push origin "$BRANCH" >> "$LOGFILE" 2>&1 || \
            git push --set-upstream origin "$BRANCH" >> "$LOGFILE" 2>&1

        gh pr create \
            --title "Dogfood: $(date +%Y-%m-%d) automated improvements" \
            --body "$(git log --oneline "$BRANCH" --not main)" \
            --base main --head "$BRANCH" >> "$LOGFILE" 2>&1 || true
        log "PR created"
    else
        log "No changes to submit"
    fi

    stop_agents
    log "=== Dogfood run complete ==="
}

# --- Main ---
case "${1:-}" in
    stop) stop_agents ;;
    *)    run "$@" ;;
esac
