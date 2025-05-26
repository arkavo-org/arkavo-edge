#!/usr/bin/env python3
"""
MCP Protocol Validation Test Script
Tests arkavo MCP server against JSON schema specifications
"""

import json
import subprocess
import sys
from typing import Dict, List, Tuple
import jsonschema

# MCP Schema based on spec
MCP_SCHEMA = {
    "InitializeResponse": {
        "type": "object",
        "properties": {
            "jsonrpc": {"const": "2.0"},
            "id": {"type": ["string", "number", "null"]},
            "result": {
                "type": "object",
                "properties": {
                    "protocolVersion": {"type": "string"},
                    "capabilities": {"type": "object"},
                    "serverInfo": {
                        "type": "object",
                        "properties": {
                            "name": {"type": "string"},
                            "version": {"type": "string"}
                        },
                        "required": ["name", "version"]
                    }
                },
                "required": ["protocolVersion", "capabilities", "serverInfo"]
            }
        },
        "required": ["jsonrpc", "id", "result"]
    },
    "ToolsListResponse": {
        "type": "object",
        "properties": {
            "jsonrpc": {"const": "2.0"},
            "id": {"type": ["string", "number", "null"]},
            "result": {
                "type": "object",
                "properties": {
                    "tools": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": {"type": "string"},
                                "description": {"type": "string"},
                                "inputSchema": {"type": "object"}
                            },
                            "required": ["name", "inputSchema"]
                        }
                    }
                },
                "required": ["tools"]
            }
        },
        "required": ["jsonrpc", "id", "result"]
    }
}

def run_mcp_command(requests: List[Dict]) -> List[Dict]:
    """Run MCP server with given requests and return responses"""
    cmd = ["cargo", "run", "--bin", "arkavo", "--", "serve"]
    input_data = "\n".join(json.dumps(req) for req in requests)
    
    proc = subprocess.Popen(
        cmd,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        cwd="/Users/paul/Projects/arkavo/arkavo-edge"
    )
    
    stdout, stderr = proc.communicate(input=input_data, timeout=30)
    
    responses = []
    for line in stdout.strip().split("\n"):
        if line and line.startswith("{"):
            try:
                responses.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    
    return responses

def validate_response(response: Dict, schema_name: str) -> Tuple[bool, List[str]]:
    """Validate response against schema"""
    schema = MCP_SCHEMA.get(schema_name)
    if not schema:
        return False, [f"Unknown schema: {schema_name}"]
    
    try:
        jsonschema.validate(instance=response, schema=schema)
        return True, []
    except jsonschema.exceptions.ValidationError as e:
        errors = [str(e)]
        for error in e.context:
            errors.append(f"  - {error.message} at {'.'.join(str(p) for p in error.path)}")
        return False, errors

def test_initialize():
    """Test initialize request/response"""
    print("Testing Initialize...")
    
    requests = [
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "test-client",
                    "version": "1.0.0"
                }
            }
        }
    ]
    
    responses = run_mcp_command(requests)
    if not responses:
        print("  ❌ No response received")
        return False
    
    valid, errors = validate_response(responses[0], "InitializeResponse")
    if valid:
        print("  ✅ Initialize response valid")
        return True
    else:
        print("  ❌ Initialize response invalid:")
        for error in errors:
            print(f"    {error}")
        return False

def test_tools_list():
    """Test tools/list request/response"""
    print("\nTesting Tools List...")
    
    requests = [
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "test-client",
                    "version": "1.0.0"
                }
            }
        },
        {
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }
    ]
    
    responses = run_mcp_command(requests)
    if len(responses) < 2:
        print("  ❌ Not enough responses received")
        return False
    
    valid, errors = validate_response(responses[1], "ToolsListResponse")
    if valid:
        tools = responses[1]["result"]["tools"]
        print(f"  ✅ Tools list valid with {len(tools)} tools:")
        for tool in tools:
            print(f"    - {tool['name']}: {tool.get('description', 'No description')}")
        
        # Check for iOS tools
        ios_tools = ["ui_interaction", "screen_capture", "ui_query"]
        found_tools = [t["name"] for t in tools]
        for ios_tool in ios_tools:
            if ios_tool in found_tools:
                print(f"    ✅ Found iOS tool: {ios_tool}")
            else:
                print(f"    ❌ Missing iOS tool: {ios_tool}")
        
        return True
    else:
        print("  ❌ Tools list response invalid:")
        for error in errors:
            print(f"    {error}")
        return False

def test_tool_call():
    """Test tool call request/response"""
    print("\nTesting Tool Call...")
    
    requests = [
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "test-client",
                    "version": "1.0.0"
                }
            }
        },
        {
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "screen_capture",
                "arguments": {
                    "name": "test_screenshot"
                }
            }
        }
    ]
    
    responses = run_mcp_command(requests)
    if len(responses) < 2:
        print("  ❌ Not enough responses received")
        return False
    
    # For now, just check structure
    response = responses[1]
    if "result" in response or "error" in response:
        print("  ✅ Tool call response received")
        return True
    else:
        print("  ❌ Invalid tool call response structure")
        return False

def main():
    print("MCP Protocol Validation Tests")
    print("=" * 50)
    
    tests = [
        test_initialize,
        test_tools_list,
        test_tool_call
    ]
    
    passed = 0
    for test in tests:
        if test():
            passed += 1
    
    print("\n" + "=" * 50)
    print(f"Results: {passed}/{len(tests)} tests passed")
    
    return 0 if passed == len(tests) else 1

if __name__ == "__main__":
    sys.exit(main())