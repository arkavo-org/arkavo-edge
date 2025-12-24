use crate::error::{Result, MemoryError};
use crate::storage::MemoryStorage;
use uuid::Uuid;

pub struct ContextLedger {
    storage: MemoryStorage,
}

impl ContextLedger {
    pub fn new(storage: MemoryStorage) -> Self {
        Self { storage }
    }

    pub async fn offload(&self, content: &str, summary: &str, source: &str) -> Result<String> {
        let id = Uuid::new_v4();
        
        self.storage.store_ledger_fragment(
            id, 
            content.to_string(), 
            summary.to_string(), 
            source.to_string()
        ).await?;
        
        Ok(format!("[ARCHIVED: {} - ID: {}]", summary, id))
    }

    pub async fn restore(&self, pointer_id: &str) -> Result<String> {
        let id = Uuid::parse_str(pointer_id).map_err(|_| MemoryError::Storage("Invalid UUID".to_string()))?;
        
        let memory = self.storage.get_ledger_fragment(id).await?;
        
        Ok(memory.content)
    }
}
