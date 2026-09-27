#!/usr/bin/env python3
"""Minimal MCP server fixture for arkavo-mcp-proxy integration tests.

Speaks line-delimited JSON-RPC over stdio. Serves the tools listed in TOOLS
and echoes the tool name, arguments, and `_meta` back from tools/call so tests
can verify pass-through content. When MCP_PROXY_TEST_RECORD is set, every tool
call that reaches tools/call is appended to that file as
"<name> <arguments json>", which lets tests prove a denied or dropped call
never arrived upstream.

Several tools exist to drive proxy behaviour that a well-behaved server cannot:

- "never_replies" returns nothing at all, so the proxy's per-request timeout
  fires and the call never reaches the tool.
- "failing_tool" answers with a JSON-RPC error, which is a completed call
  whose result happens to be a failure.
- "server_request" sends a server-initiated request (the shape of
  sampling/createMessage) and reports whatever the proxy answers, which is how
  a test observes the proxy's reply to traffic in that direction.
- "id_collision" does the same with the id of the tools/call it is answering,
  the hostile move a proxy that matched by id before checking the shape would
  relay to the client as if it were this tool's result.
- "over_long_line" writes a line past the proxy's 1 MiB frame cap before its
  real response, so a test can see the cap discard it and the call still work.
- "refusal_flood" writes many unmatched server-initiated requests before its
  real response, so a test can see that answering them never stalls the
  proxy's reading of the response itself.

Setting MCP_PROXY_TEST_STALL_STDIN makes the server stop reading its stdin
once the handshake is done: it answers "initialize" and then sleeps forever
without reading another byte. A message large enough to fill the pipe then
blocks the proxy's write, which is the case its write timeout exists for.
"""

import json
import os
import sys
import time

RECORD_FILE = os.environ.get("MCP_PROXY_TEST_RECORD")
STALL_STDIN = os.environ.get("MCP_PROXY_TEST_STALL_STDIN")

# Long enough that any write timeout a test sets fires first, short enough
# that a fixture left behind by a failed test does not linger.
STALL_SECONDS = 300

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
    {
        "name": "never_replies",
        "description": "Sends no response, so the proxy's request times out",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "failing_tool",
        "description": "Answers with a JSON-RPC error: a completed, failed call",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "server_request",
        "description": "Asks the client something and reports the answer",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "id_collision",
        "description": "Asks the client something reusing this call's own id",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "over_long_line",
        "description": "Writes a line past the proxy's frame cap, then answers",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "refusal_flood",
        "description": "Writes many unmatched requests, then answers",
        "inputSchema": {"type": "object", "properties": {}},
    },
]

# Past the proxy's 1 MiB downstream/upstream line cap.
OVER_LONG_LINE_BYTES = 1024 * 1024 + 1

# Comfortably more than the proxy's refusal queue, and enough traffic that a
# proxy writing the refusals inline would fill the pipe it writes them to,
# stop reading, and so block this fixture's own stdout before it ever gets
# back to reading them: a deadlock the queue is what avoids.
REFUSAL_FLOOD_COUNT = 2000


def record(tool_name: str) -> None:
    if RECORD_FILE:
        with open(RECORD_FILE, "a", encoding="utf-8") as handle:
            handle.write(tool_name + "\n")


def send(message: dict) -> None:
    print(json.dumps(message), flush=True)


def ask_the_client(request_id: object = "server-initiated-1") -> dict:
    """Send a server-initiated request and read whatever comes back.

    MCP allows a server to ask the client for something mid-call
    (sampling/createMessage and friends). This proxy does not relay those, so
    the reply read here is the proxy's own.

    `request_id` is the server's to choose, which is exactly why it is a
    parameter: a hostile server picks the id of a call already in flight.
    """
    send(
        {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "sampling/createMessage",
            "params": {"messages": []},
        }
    )
    line = sys.stdin.readline()
    if not line:
        return {"error": "no reply: the connection closed"}
    try:
        return json.loads(line)
    except json.JSONDecodeError:
        return {"error": "reply was not JSON"}


def tool_result(content: object, meta: object) -> dict:
    return {
        "content": [{"type": "text", "text": json.dumps(content)}],
        "meta": meta,
    }


def main() -> None:
    while True:
        line = sys.stdin.readline()
        if not line:
            break
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            continue

        method = request.get("method")
        req_id = request.get("id")

        if method is None:
            # An answer to something this server asked for, read after the
            # tool that asked stopped waiting for it. Never a request.
            continue

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
            if STALL_STDIN:
                # Read nothing more, ever. The pipe the proxy writes to fills
                # and its write blocks, which is what the write timeout is
                # there to bound.
                time.sleep(STALL_SECONDS)
                return
            continue
        elif method == "tools/list":
            response = {"jsonrpc": "2.0", "id": req_id, "result": {"tools": TOOLS}}
        elif method == "tools/call":
            params = request.get("params") or {}
            name = params.get("name", "")
            arguments = params.get("arguments", {})
            record(f"{name} {json.dumps(arguments)}")
            if name == "never_replies":
                continue
            if name == "failing_tool":
                response = {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": {"code": -32000, "message": "the tool itself failed"},
                }
            elif name == "server_request":
                reply = ask_the_client()
                response = {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": tool_result(
                        {"tool": name, "server_request_reply": reply},
                        params.get("_meta"),
                    ),
                }
            elif name == "id_collision":
                # The id of the call being answered, so a proxy that decided
                # by id before shape would hand this to the waiting caller.
                reply = ask_the_client(req_id)
                record(f"id_collision reply {json.dumps(reply)}")
                response = {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": tool_result(
                        {"tool": name, "server_request_reply": reply},
                        params.get("_meta"),
                    ),
                }
            elif name == "over_long_line":
                print("x" * OVER_LONG_LINE_BYTES, flush=True)
                response = {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": tool_result({"tool": name}, params.get("_meta")),
                }
            elif name == "refusal_flood":
                for index in range(REFUSAL_FLOOD_COUNT):
                    send(
                        {
                            "jsonrpc": "2.0",
                            "id": f"flood-{index}",
                            "method": "sampling/createMessage",
                            "params": {"messages": []},
                        }
                    )
                response = {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": tool_result({"tool": name}, params.get("_meta")),
                }
            else:
                response = {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": tool_result(
                        {"tool": name, "arguments": arguments},
                        params.get("_meta"),
                    ),
                }
        else:
            response = {
                "jsonrpc": "2.0",
                "id": req_id,
                "error": {"code": -32601, "message": f"unknown method: {method}"},
            }

        send(response)


if __name__ == "__main__":
    main()
