pub struct A2aClient;

impl Default for A2aClient {
    fn default() -> Self {
        return Self::new();
    }
}

impl A2aClient {
    pub const fn new() -> Self {
        Self
    }

    pub fn send(&self, _message: &str) -> Result<String, Box<dyn std::error::Error>> {
        return Ok("A2A response".to_string());
    }
}
