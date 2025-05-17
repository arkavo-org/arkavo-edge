pub mod mcp;
pub mod a2a;

pub struct Client;

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Self {
        Client
    }
    
    pub fn send_message(&self, message: &str) -> Result<String, Box<dyn std::error::Error>> {
        Ok(format!("Response to: {}", message))
    }
}