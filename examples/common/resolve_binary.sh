#!/bin/bash
# resolve_binary.sh - Locate the arkavo executable for the example scripts
#
# Usage: source this file, then call one of:
#   require_arkavo_binary        # sets BINARY, or prints why not and exits 1
#   resolve_arkavo_binary        # sets BINARY, or sets it to "" and returns 1
#
# Resolution order:
#   1. $BINARY (or $ARKAVO_BIN) when set. An explicit choice is never
#      silently replaced: if it does not point at an executable, resolution fails.
#   2. The source build: target/debug/arkavo, then target/release/arkavo.
#   3. arkavo on PATH (Homebrew, .pkg, .deb, or an unpacked release archive).
#
# The examples run against an installed binary, so a source build is needed
# only when no arkavo is installed or when testing local changes.

ARKAVO_EXAMPLES_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ARKAVO_BINARY_REQUESTED=""

_arkavo_is_executable_file() {
    [ -n "$1" ] && [ -f "$1" ] && [ -x "$1" ]
}

resolve_arkavo_binary() {
    local candidate
    ARKAVO_BINARY_REQUESTED="${BINARY:-${ARKAVO_BIN:-}}"

    if [ -n "$ARKAVO_BINARY_REQUESTED" ]; then
        candidate="$ARKAVO_BINARY_REQUESTED"
        # A bare command name (no slash) is looked up on PATH.
        case "$candidate" in
            */*) ;;
            *) candidate="$(command -v "$candidate" 2>/dev/null || true)" ;;
        esac
        if _arkavo_is_executable_file "$candidate"; then
            BINARY="$candidate"
            return 0
        fi
        BINARY=""
        return 1
    fi

    for candidate in \
        "$ARKAVO_EXAMPLES_ROOT/target/debug/arkavo" \
        "$ARKAVO_EXAMPLES_ROOT/target/release/arkavo"; do
        if _arkavo_is_executable_file "$candidate"; then
            BINARY="$candidate"
            return 0
        fi
    done

    candidate="$(command -v arkavo 2>/dev/null || true)"
    if _arkavo_is_executable_file "$candidate"; then
        BINARY="$candidate"
        return 0
    fi

    BINARY=""
    return 1
}

explain_missing_arkavo_binary() {
    {
        echo "Error: arkavo binary not found."
        if [ -n "$ARKAVO_BINARY_REQUESTED" ]; then
            echo "  BINARY is set to '$ARKAVO_BINARY_REQUESTED', which is not an executable file."
            echo "  Point BINARY at an arkavo executable, or unset it to use the default lookup."
        else
            echo "  Looked for:"
            echo "    $ARKAVO_EXAMPLES_ROOT/target/debug/arkavo"
            echo "    $ARKAVO_EXAMPLES_ROOT/target/release/arkavo"
            echo "    arkavo on PATH"
            echo "  Install arkavo (for example: brew install arkavo), build from source"
            echo "  with 'cargo build', or set BINARY=/path/to/arkavo."
        fi
    } >&2
}

require_arkavo_binary() {
    if ! resolve_arkavo_binary; then
        explain_missing_arkavo_binary
        exit 1
    fi
}
