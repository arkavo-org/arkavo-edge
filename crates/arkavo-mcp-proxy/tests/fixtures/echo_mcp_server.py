#!/usr/bin/env python3
"""Minimal MCP server fixture for arkavo-mcp-proxy integration tests.

Speaks line-delimited JSON-RPC over stdio. Serves two tools ("echo" and
"blocked_tool") and echoes the tool name and arguments back from tools/call
so tests can verify pass-through content. When MCP_PROXY_TEST_RECORD is set,
every tool name that reaches tools/call is appended to that file, which lets
tests prove a denied call never arrived upstream.
"""

import json
import os
import sys

RECORD_FILE = os.environ.get("MCP_PROXY_TEST_RECORD")

TOOLS = [
    {
        "name": "echo",
        "description": "Echo back the call name and arguments",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "blocked_tool",
        "description": "Denied by the proxy policy in tests",
        "inputSchema": {"type": "object", "properties": {}},
    },
]


def record(tool_name: str) -> None:
    if RECORD_FILE:
        with open(RECORD_FILE, "a", encoding="utf-8") as handle:
            handle.write(tool_name + "\n")


def main() -> None:
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            continue

        method = request.get("method")
        req_id = request.get("id")

        if method == "initialize":
            response = {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "protocolVersion": "2025-11-25",
                    "serverInfo": {"name": "echo-mcp-server", "version": "0.1.0"},
                    "capabilities": {"tools": {}},
                },
            }
        elif method == "notifications/initialized":
            continue
        elif method == "tools/list":
            response = {"jsonrpc": "2.0", "id": req_id, "result": {"tools": TOOLS}}
        elif method == "tools/call":
            params = request.get("params") or {}
            name = params.get("name", "")
            record(name)
            response = {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [
                        {
                            "type": "text",
                            "text": json.dumps(
                                {
                                    "tool": name,
                                    "arguments": params.get("arguments", {}),
                                }
                            ),
                        }
                    ]
                },
            }
        else:
            response = {
                "jsonrpc": "2.0",
                "id": req_id,
                "error": {"code": -32601, "message": f"unknown method: {method}"},
            }

        print(json.dumps(response), flush=True)


if __name__ == "__main__":
    main()
