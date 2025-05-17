pub struct Vault {
    path: String,
}

impl Vault {
    pub fn new(path: &str) -> Self {
        Vault {
            path: path.to_string(),
        }
    }
    
    pub fn import(&self, _content: &str) -> Result<(), Box<dyn std::error::Error>> {
        println!("Importing to vault at {}", self.path);
        Ok(())
    }
    
    pub fn export(&self) -> Result<String, Box<dyn std::error::Error>> {
        println!("Exporting from vault at {}", self.path);
        Ok("Exported content".to_string())
    }
}