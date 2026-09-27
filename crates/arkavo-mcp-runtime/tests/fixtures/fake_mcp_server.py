#!/usr/bin/env python3
"""Minimal fake MCP server that speaks JSON-RPC over stdio.

Responds to the handshake (initialize / notifications/initialized) and to
tools/list so the runtime's McpClient can complete a connection.  All output
is flushed line-by-line so the line-oriented stdio transport sees complete
responses immediately.
"""

import json
import sys


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
                    "serverInfo": {"name": "fake-mcp-server", "version": "0.1.0"},
                    "capabilities": {},
                },
            }
        elif method == "notifications/initialized":
            continue
        elif method == "tools/list":
            response = {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "tools": [
                        {
                            "name": "echo",
                            "description": "Echo back the input parameters",
                            "inputSchema": {"type": "object", "properties": {}},
                        }
                    ]
                },
            }
        elif method == "tools/call":
            response = {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {"content": [{"type": "text", "text": "done"}]},
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
