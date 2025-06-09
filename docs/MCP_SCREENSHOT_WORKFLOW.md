# MCP Screenshot Workflow Guide

This guide explains how to use the MCP (Model Context Protocol) tools in arkavo chat to take screenshots and analyze them using vision capabilities.

## Prerequisites

1. Start the MCP server in one terminal:
   ```bash
   arkavo serve
   ```

2. Have an iOS simulator running

## Basic Usage

### Interactive Mode

Start an interactive chat session:
```bash
arkavo chat
```

Then you can use natural language or direct tool calls:

1. **Natural language**: "Take a screenshot of the iOS simulator"
2. **Direct tool call**: `@device_management {"action": "list"}`

### Command Line Mode

Use `--prompt` (or `--print` for compatibility) for single commands:

```bash
# List available devices
arkavo chat --prompt "@device_management {\"action\": \"list\"}"

# Take a screenshot (natural language)
arkavo chat --prompt "Take a screenshot of the iOS simulator and describe what you see"

# Direct screenshot with known device ID
arkavo chat --prompt "@screen_capture {\"device_id\": \"YOUR-DEVICE-ID\"}"
```

## Typical Workflow

The assistant will automatically:

1. **Find devices**: Use `@device_management {"action": "list"}` to get available device IDs
2. **Capture screen**: Use `@screen_capture {"device_id": "..."}` to take a screenshot
3. **Analyze image**: The screenshot path is returned and can be analyzed using vision capabilities

## Tool Syntax

Tools can be invoked in two ways:

1. **JSON arguments**: `@tool_name {"param": "value"}`
2. **Plain text**: `@tool_name plain text arguments` (converted to `{"prompt": "plain text arguments"}`)

## Available Tools for Screenshot Workflow

- `device_management`: List and manage iOS devices
- `screen_capture`: Take screenshots of devices
- `ui_interaction`: Tap, swipe, or enter text based on coordinates
- `ui_query`: Query UI elements (requires XCUITest setup)

## Example Session

```bash
$ arkavo chat
> Take a screenshot and tell me what you see