# CEF Error Reporting to LLM

This document describes the comprehensive error reporting system implemented for the CEF-based UI renderer that captures browser errors and makes them available to the LLM for feedback and correction.

## Overview

The error reporting system captures HTML, JavaScript, console, and network errors from the CEF browser and transports them through a Unix Domain Socket to the Rust layer, where they can be displayed to the user and fed back to the LLM for iterative correction.

## Architecture

```
Browser (CEF/Chromium)
  ├─ Console Messages (console.error, warnings)
  ├─ Load Errors (network, DNS, 404s)
  └─ JavaScript Exceptions
         │
         ▼
  browser_client.cc handlers
  (OnConsoleMessage, OnLoadError)
         │
         ▼
  DOMExecutor::SendError()
         │
         ▼
  UdsClient (Unix Socket)
  Message Type: 0x03
         │
         ▼
  Rust UDS Transport
  (protocol::DOMError)
         │
         ▼
  CefRenderer::try_recv_error()
         │
         ▼
  UI Event Loop (ui.rs)
         │
    ┌────┴────┐
    ▼         ▼
Display    Log to
in UI      Console
```

## Error Types

### DOMError Structure

The `DOMError` struct is defined identically in both C++ and Rust:

```rust
pub struct DOMError {
    pub error_type: String,  // "console" | "load"
    pub severity: String,     // "error" | "warning" | "info"
    pub message: String,      // Human-readable error message
    pub source: String,       // Source file or URL
    pub line: u32,           // Line number (0 if not applicable)
}
```

### Console Errors (error_type: "console")

Captured via `CefDisplayHandler::OnConsoleMessage`:

- `console.error()` messages from JavaScript
- `console.warn()` warnings
- `console.info()` informational messages
- JavaScript exceptions and runtime errors
- HTML parsing warnings

**Example:**
```json
{
  "error_type": "console",
  "severity": "error",
  "message": "Uncaught ReferenceError: undefinedFunction is not defined",
  "source": "data:text/html",
  "line": 12
}
```

### Load Errors (error_type: "load")

Captured via `CefLoadHandler::OnLoadError`:

- Network connection failures
- HTTP errors (404, 500, etc.)
- CORS violations
- DNS resolution failures
- SSL/TLS certificate errors
- Resource load failures

**Example:**
```json
{
  "error_type": "load",
  "severity": "error",
  "message": "Failed to load: net::ERR_CONNECTION_REFUSED",
  "source": "https://example.com/api/data",
  "line": 0
}
```

## Protocol

### Message Format

Errors are transmitted over Unix Domain Socket using a custom framed protocol:

**Frame Structure:**
```
[4 bytes: frame_len (little-endian)]
[frame_len bytes: message data]
```

**Message Structure (Type 0x03):**
```
[1 byte: 0x03 (error message type)]
[4 bytes: error_type length]
[N bytes: error_type string]
[4 bytes: severity length]
[N bytes: severity string]
[4 bytes: message length]
[N bytes: message string]
[4 bytes: source length]
[N bytes: source string]
[4 bytes: line number (little-endian)]
```

## Implementation Details

### C++ Layer

**browser_client.cc:**
```cpp
bool ArkavoBrowserClient::OnConsoleMessage(
    CefRefPtr<CefBrowser> browser,
    cef_log_severity_t level,
    const CefString& message,
    const CefString& source,
    int line) {

    if (level == LOGSEVERITY_ERROR) {
        DOMError error;
        error.error_type = "console";
        error.severity = "error";
        error.message = message.ToString();
        error.source = source.ToString();
        error.line = static_cast<uint32_t>(line);

        DOMExecutor::GetInstance()->SendError(error);
    }
    return false;
}
```

**uds_client.cc:**
```cpp
bool UdsClient::SendError(const DOMError& error) {
    uint8_t buffer[2048];
    uint32_t offset = 0;

    buffer[offset++] = 0x03; // Message type

    // Serialize strings and line number
    write_string(error.error_type);
    write_string(error.severity);
    write_string(error.message);
    write_string(error.source);

    memcpy(buffer + offset, &error.line, sizeof(error.line));
    offset += sizeof(error.line);

    // Send framed message
    // ...
}
```

### Rust Layer

**protocol.rs:**
```rust
pub fn deserialize_error(data: &[u8]) -> Result<DOMError> {
    if data[0] != 0x03 {
        return Err(CefError::ProtocolError("Invalid error message type"));
    }

    let mut cursor = 1;
    let error_type = read_string(&mut cursor)?;
    let severity = read_string(&mut cursor)?;
    let message = read_string(&mut cursor)?;
    let source = read_string(&mut cursor)?;
    let line = u32::from_le_bytes([...]);

    Ok(DOMError { error_type, severity, message, source, line })
}
```

**dom_commands.rs:**
```rust
pub async fn try_recv_error(&mut self) -> Result<Option<DOMError>> {
    match timeout(Duration::from_millis(10), self.transport.recv_error()).await {
        Ok(Ok(error)) => Ok(Some(error)),
        Ok(Err(e)) => Err(e),
        Err(_) => Ok(None),
    }
}
```

**ui.rs Event Loop:**
```rust
loop {
    // Poll for errors from CEF
    if let Ok(Some(error)) = cef_renderer.try_recv_error().await {
        eprintln!(
            "[CEF {} Error] {}: {} ({}:{})",
            error.error_type, error.severity,
            error.message, error.source, error.line
        );

        // Display error in UI with red styling
        let error_html = format!(...);
        let _ = cef_renderer.render(&error_html, "", "").await;
    }

    // Poll for events...
}
```

## Error Display

Errors are displayed in the UI with red styling:

```html
<div style="padding: 40px; font-family: system-ui;">
    <div style="background: #fee; border-left: 4px solid #f44; padding: 16px;">
        <strong style="color: #c33;">Error: console</strong><br>
        <span style="color: #666;">Uncaught ReferenceError: undefinedFunction is not defined</span><br>
        <small style="color: #999;">data:text/html:12</small>
    </div>
</div>
```

## Testing

### Manual Testing

Create a test HTML file with errors:

```html
<!DOCTYPE html>
<html>
<head><title>Error Test</title></head>
<body>
    <div id="content">Test</div>
    <script>
        console.error("Test error from JavaScript");
        undefinedFunction(); // Triggers error
    </script>
</body>
</html>
```

Run the UI command:

```bash
ARKAVO_CEF_RENDERER_PATH=./target/debug/build/arkavo-cef-*/out/bin/arkavo-cef-renderer.app/Contents/MacOS/arkavo-cef-renderer \
./target/debug/arkavo ui --prompt "Show test content"
```

Expected output:

```
[CEF console Error] error: Test error from JavaScript (data:text/html:6)
[CEF console Error] error: Uncaught ReferenceError: undefinedFunction is not defined (data:text/html:7)
```

### Integration Tests

See `crates/arkavo-agui/tests/cef_error_handling_test.rs` for automated tests covering:

- Console message capture
- Load error handling
- Invalid HTML detection
- XSS payload sanitization

## Future: LLM Feedback Loop

To complete the error feedback system for LLM-based UI generation:

### Step 1: Buffer Errors During Rendering

Add error buffering to the renderer state:

```rust
pub struct ErrorBuffer {
    errors: Vec<DOMError>,
    max_errors: usize,
}

impl ErrorBuffer {
    pub fn add(&mut self, error: DOMError) {
        if self.errors.len() < self.max_errors {
            self.errors.push(error);
        }
    }

    pub fn format_for_llm(&self) -> String {
        self.errors.iter()
            .map(|e| format!("{}:{} - {}", e.source, e.line, e.message))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn clear(&mut self) {
        self.errors.clear();
    }
}
```

### Step 2: Include Errors in LLM Context

Modify `handle_prompt()` to include recent errors:

```rust
async fn handle_prompt(
    renderer: &mut dyn UiRenderer,
    prompt: &str,
    error_buffer: &ErrorBuffer,
) -> Result<()> {
    let enhanced_prompt = if !error_buffer.errors.is_empty() {
        format!(
            "Previous UI rendering had the following errors:\n\n{}\n\n\
             Please fix these issues and regenerate.\n\n\
             User request: {}",
            error_buffer.format_for_llm(),
            prompt
        )
    } else {
        prompt.to_string()
    };

    // Call LLM with enhanced_prompt...
    // If successful render, error_buffer.clear()
}
```

### Step 3: Iterative Correction

The LLM receives context like:

```
Previous UI rendering had the following errors:

data:text/html:12 - Unclosed div tag
data:text/html:15 - Invalid CSS property: colr (did you mean: color?)
data:text/html:20 - Uncaught ReferenceError: onClick is not defined

Please fix these issues and regenerate.

User request: Create a blue button that shows an alert
```

The LLM can then generate corrected HTML/CSS/JS based on the error feedback.

## Files Modified

### Rust Layer
- `crates/arkavo-cef/src/protocol.rs` - Error types & deserialization
- `crates/arkavo-cef/src/uds.rs` - Error transport & `ReceivedMessage::Error`
- `crates/arkavo-cef/src/dom_commands.rs` - `try_recv_error()` API
- `crates/arkavo-cef/src/lib.rs` - Public `DOMError` export
- `crates/arkavo-agui/src/renderer/cef_renderer.rs` - Error polling
- `crates/arkavo-cli/src/commands/ui.rs` - Error display

### C++ Layer
- `crates/arkavo-cef/cef-bridge/uds_client.h` - `DOMError` struct & `SendError()`
- `crates/arkavo-cef/cef-bridge/uds_client.cc` - Error serialization
- `crates/arkavo-cef/cef-bridge/browser_client.h` - Handler declarations
- `crates/arkavo-cef/cef-bridge/browser_client.cc` - `OnConsoleMessage()` & `OnLoadError()`
- `crates/arkavo-cef/cef-bridge/dom_executor.h` - Public `SendError()`
- `crates/arkavo-cef/cef-bridge/dom_executor.cc` - Error forwarding

## Performance Considerations

- Error polling uses a 10ms timeout to avoid blocking
- Non-blocking event loop checks errors every 100ms
- Errors are transmitted over Unix Domain Socket (low latency)
- HTML escaping is applied to prevent XSS in error display
- Load errors filter out intentional aborts (ERR_ABORTED)

## Security

- Socket paths are validated (must start with `/tmp/arkavo_`, `/private/tmp/arkavo_`, or `/var/folders/`)
- Socket permissions are set to 0600 (owner only)
- Error messages are HTML-escaped before display
- Error buffer could implement size limits to prevent memory exhaustion

## Limitations

- JavaScript exceptions in `ExecuteJavaScript()` are not directly captured (would require V8 context integration)
- Some browser internal errors may not surface through CEF handlers
- Error line numbers are relative to the injected HTML, not the original LLM output

## References

- CEF Display Handler: https://bitbucket.org/chromiumembedded/cef/wiki/GeneralUsage#markdown-header-cef-display-handler
- CEF Load Handler: https://bitbucket.org/chromiumembedded/cef/wiki/GeneralUsage#markdown-header-cef-load-handler
- Unix Domain Sockets in Rust: https://docs.rs/tokio/latest/tokio/net/struct.UnixStream.html
