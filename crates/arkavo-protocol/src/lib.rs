pub mod a2a;
pub mod mcp;

pub struct Client;

impl Default for Client {
    fn default() -> Self {
        return Self::new();
    }
}

impl Client {
    pub const fn new() -> Self {
        Self
    }

    pub fn send_message(&self, message: &str) -> Result<String, Box<dyn std::error::Error>> {
        return Ok(format!("Response to: {message}"));
    }
}
