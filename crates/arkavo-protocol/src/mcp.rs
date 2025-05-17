pub struct McpClient;

impl Default for McpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl McpClient {
    pub fn new() -> Self {
        McpClient
    }
    
    pub fn send(&self, _message: &str) -> Result<String, Box<dyn std::error::Error>> {
        Ok("MCP response".to_string())
    }
}