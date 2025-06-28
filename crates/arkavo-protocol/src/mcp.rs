pub struct McpClient;

impl Default for McpClient {
    fn default() -> Self {
        return Self::new();
    }
}

impl McpClient {
    pub const fn new() -> Self {
        Self
    }

    pub fn send(&self, _message: &str) -> Result<String, Box<dyn std::error::Error>> {
        return Ok("MCP response".to_string());
    }
}
