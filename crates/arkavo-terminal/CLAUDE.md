# CLAUDE.md - Arkavo Terminal UI

Guidance for AI assistants working on the Arkavo Terminal UI, focused on preventing recurring TUI bugs.

## Key Bindings

### Global Navigation
- `Tab` / `Shift+Tab` - Cycle between view modes
- `Ctrl+q` - Quit application

### Vim Mode
- `i` - Enter Insert mode
- `Esc` - Return to Normal mode
- `v` - Enter Visual mode
- `:` - Enter Command mode
- `h/j/k/l` - Navigation (left/down/up/right)
- `g/G` - Go to top/bottom
- `Ctrl+u/d` - Page up/down
- `y` - Yank (copy) in Visual mode
- `p` - Paste

### Chat View
- `Enter` - Send message (Insert mode)
- `m` - Open model selection
- `Up/Down` - Navigate model list
- `Esc` - Cancel model selection

### Code View
- `Ctrl+e` - Open external editor (Helix)
- Standard vim navigation applies

## TUI Testing Tools

MCP tools for automated TUI testing (`tui_keyboard`, `tui_screenshot`, `tui_interaction`, `tui_harness`) live in `crates/arkavo-mcp-macos/src/mcp/tui_*`. Regression tests are in `tests/`.

## Common Bug Patterns and Prevention

### 1. Mode Confusion
**Issue**: Vim mode state not properly reflected in UI
**Prevention**: 
- Always update `vim_state` before rendering
- Ensure mode indicator is visible in status line
- Test mode transitions with screenshot verification

### 2. Focus Loss
**Issue**: Focus jumps unexpectedly between panes
**Prevention**:
- Centralize focus management in `App::handle_focus_change()`
- Validate focus target before switching
- Test all focus transitions in both layout modes

### 3. Model Connection Drops
**Issue**: Model connections silently fail
**Prevention**:
- Implement connection health checks
- Display connection status in UI
- Provide clear error messages with recovery options

### 4. Streaming State Corruption
**Issue**: Multiple streaming responses overlap
**Prevention**:
- Use task_id to track individual requests
- Cancel previous streams before starting new ones
- Test concurrent model requests

### 5. Key Binding Conflicts
**Issue**: Keys perform unexpected actions
**Prevention**:
- Document all key bindings in this file
- Check for conflicts when adding new bindings
- Test key combinations with modifiers

## Performance Considerations

1. **Frame Budget**: Target <8ms render time
2. **Event Processing**: Non-blocking with 10ms tick rate
3. **Syntax Highlighting**: Lazy loading, cache parsed results
4. **Scrolling**: Virtualized rendering for large content

## Testing Checklist

Before committing TUI changes:

- [ ] Run TUI regression tests using MCP tools
- [ ] Test all view modes (Chat, Code, Diff, Debug, Dataflow)
- [ ] Test both layout modes (Tabbed, Portrait)
- [ ] Verify vim mode transitions
- [ ] Test model selection and switching
- [ ] Verify focus management
- [ ] Test error conditions (connection loss, invalid input)
- [ ] Check performance metrics in Debug view
- [ ] Test on both macOS and Linux
