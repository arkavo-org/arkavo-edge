use crate::{Result, TestError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotTree {
    pub root: SnapshotNode,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotNode {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub children: Vec<String>,
    pub data: Vec<u8>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub tags: Vec<String>,
}

pub struct SnapshotManager {
    nodes: Arc<RwLock<HashMap<String, SnapshotNode>>>,
    current_branch: Arc<RwLock<String>>,
}

impl SnapshotManager {
    pub fn new() -> Self {
        let root_id = uuid::Uuid::new_v4().to_string();
        let root_node = SnapshotNode {
            id: root_id.clone(),
            name: "root".to_string(),
            parent_id: None,
            children: Vec::new(),
            data: Vec::new(),
            timestamp: chrono::Utc::now(),
            tags: vec!["root".to_string()],
        };
        
        let mut nodes = HashMap::new();
        nodes.insert(root_id.clone(), root_node);
        
        Self {
            nodes: Arc::new(RwLock::new(nodes)),
            current_branch: Arc::new(RwLock::new(root_id)),
        }
    }
    
    pub fn create_branch(&self, name: &str, data: Vec<u8>) -> Result<String> {
        let current_id = self.current_branch.read()
            .map_err(|e| TestError::Execution(format!("Failed to read current branch: {}", e)))?
            .clone();
        
        let new_id = uuid::Uuid::new_v4().to_string();
        let new_node = SnapshotNode {
            id: new_id.clone(),
            name: name.to_string(),
            parent_id: Some(current_id.clone()),
            children: Vec::new(),
            data,
            timestamp: chrono::Utc::now(),
            tags: Vec::new(),
        };
        
        let mut nodes = self.nodes.write()
            .map_err(|e| TestError::Execution(format!("Failed to write nodes: {}", e)))?;
        
        if let Some(parent) = nodes.get_mut(&current_id) {
            parent.children.push(new_id.clone());
        }
        
        nodes.insert(new_id.clone(), new_node);
        
        Ok(new_id)
    }
    
    pub fn checkout(&self, snapshot_id: &str) -> Result<()> {
        let nodes = self.nodes.read()
            .map_err(|e| TestError::Execution(format!("Failed to read nodes: {}", e)))?;
        
        if !nodes.contains_key(snapshot_id) {
            return Err(TestError::Execution(format!("Snapshot not found: {}", snapshot_id)));
        }
        
        drop(nodes);
        
        let mut current = self.current_branch.write()
            .map_err(|e| TestError::Execution(format!("Failed to write current branch: {}", e)))?;
        
        *current = snapshot_id.to_string();
        
        Ok(())
    }
    
    pub fn merge_branches(&self, source_id: &str, target_id: &str) -> Result<String> {
        let nodes = self.nodes.read()
            .map_err(|e| TestError::Execution(format!("Failed to read nodes: {}", e)))?;
        
        let source = nodes.get(source_id)
            .ok_or_else(|| TestError::Execution(format!("Source snapshot not found: {}", source_id)))?
            .clone();
        
        let target = nodes.get(target_id)
            .ok_or_else(|| TestError::Execution(format!("Target snapshot not found: {}", target_id)))?
            .clone();
        
        drop(nodes);
        
        let merged_data = self.merge_data(&source.data, &target.data)?;
        
        let merged_id = uuid::Uuid::new_v4().to_string();
        let merged_node = SnapshotNode {
            id: merged_id.clone(),
            name: format!("merge_{}", chrono::Utc::now().timestamp()),
            parent_id: Some(target_id.to_string()),
            children: Vec::new(),
            data: merged_data,
            timestamp: chrono::Utc::now(),
            tags: vec!["merge".to_string()],
        };
        
        let mut nodes = self.nodes.write()
            .map_err(|e| TestError::Execution(format!("Failed to write nodes: {}", e)))?;
        
        if let Some(target) = nodes.get_mut(target_id) {
            target.children.push(merged_id.clone());
        }
        
        nodes.insert(merged_id.clone(), merged_node);
        
        Ok(merged_id)
    }
    
    pub fn get_history(&self, snapshot_id: &str) -> Result<Vec<SnapshotNode>> {
        let nodes = self.nodes.read()
            .map_err(|e| TestError::Execution(format!("Failed to read nodes: {}", e)))?;
        
        let mut history = Vec::new();
        let mut current_id = Some(snapshot_id.to_string());
        
        while let Some(id) = current_id {
            if let Some(node) = nodes.get(&id) {
                history.push(node.clone());
                current_id = node.parent_id.clone();
            } else {
                break;
            }
        }
        
        history.reverse();
        Ok(history)
    }
    
    pub fn tag_snapshot(&self, snapshot_id: &str, tag: &str) -> Result<()> {
        let mut nodes = self.nodes.write()
            .map_err(|e| TestError::Execution(format!("Failed to write nodes: {}", e)))?;
        
        let node = nodes.get_mut(snapshot_id)
            .ok_or_else(|| TestError::Execution(format!("Snapshot not found: {}", snapshot_id)))?;
        
        if !node.tags.contains(&tag.to_string()) {
            node.tags.push(tag.to_string());
        }
        
        Ok(())
    }
    
    pub fn find_by_tag(&self, tag: &str) -> Result<Vec<SnapshotNode>> {
        let nodes = self.nodes.read()
            .map_err(|e| TestError::Execution(format!("Failed to read nodes: {}", e)))?;
        
        let tagged: Vec<SnapshotNode> = nodes.values()
            .filter(|node| node.tags.contains(&tag.to_string()))
            .cloned()
            .collect();
        
        Ok(tagged)
    }
    
    fn merge_data(&self, _source: &[u8], target: &[u8]) -> Result<Vec<u8>> {
        Ok(target.to_vec())
    }
}

impl Default for SnapshotManager {
    fn default() -> Self {
        Self::new()
    }
}