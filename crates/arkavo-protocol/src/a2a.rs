pub struct A2aClient;

impl Default for A2aClient {
    fn default() -> Self {
        Self::new()
    }
}

impl A2aClient {
    pub fn new() -> Self {
        A2aClient
    }
    
    pub fn send(&self, _message: &str) -> Result<String, Box<dyn std::error::Error>> {
        Ok("A2A response".to_string())
    }
}