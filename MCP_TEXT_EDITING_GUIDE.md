# MCP Text Editing Guide for AI Agents

## Correct Usage of Text Editing Actions

### To Clear a Text Field:

**WRONG:**
```json
{
  "action": "type_text",
  "value": "clear_text"
}
```

**CORRECT:**
```json
{
  "action": "clear_text",
  "device_id": "YOUR-DEVICE-ID"
}
```

### To Delete Characters:

**WRONG:**
```json
{
  "action": "type_text", 
  "value": "delete_key(12)"
}
```

**CORRECT:**
```json
{
  "action": "delete_key",
  "count": 12,
  "device_id": "YOUR-DEVICE-ID"
}
```

### Complete Workflow Example:

1. **Tap on text field:**
```json
{
  "action": "tap",
  "target": {"x": 232, "y": 465},
  "device_id": "132B1310-2AF5-45F4-BB8E-CA5A2FEB9481"
}
```

2. **Clear existing text:**
```json
{
  "action": "clear_text",
  "device_id": "132B1310-2AF5-45F4-BB8E-CA5A2FEB9481"
}
```

3. **Type new text:**
```json
{
  "action": "type_text",
  "value": "unique_user_2025",
  "device_id": "132B1310-2AF5-45F4-BB8E-CA5A2FEB9481"
}
```

## Available Text Editing Actions:

- `clear_text` - Selects all and deletes (Cmd+A then Delete)
- `select_all` - Selects all text (Cmd+A)
- `delete_key` - Presses delete key (use with `count` parameter)
- `copy` - Copies selected text (Cmd+C)
- `paste` - Pastes from clipboard (Cmd+V)

## Important Notes:

1. These are separate ACTIONS, not values to pass to type_text
2. Always tap on a text field first to focus it
3. Use clear_text instead of multiple delete keys
4. DO NOT use idb commands - use MCP tools only