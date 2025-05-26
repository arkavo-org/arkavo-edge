#[cfg(test)]
mod tests {
    use super::super::state_store::StateStore;
    use serde_json::json;
    
    #[test]
    fn test_basic_state_operations() {
        let store = StateStore::new();
        
        // Test set and get
        let data = json!({"name": "test", "value": 42});
        store.set("entity1", data.clone()).unwrap();
        
        let retrieved = store.get("entity1").unwrap();
        assert_eq!(retrieved, Some(data));
        
        // Test get non-existent
        let missing = store.get("missing").unwrap();
        assert_eq!(missing, None);
    }
    
    #[test]
    fn test_update_operations() {
        let store = StateStore::new();
        
        // Create initial state
        store.set("user", json!({"name": "Alice", "age": 30})).unwrap();
        
        // Update with merge
        let updated = store.update("user", "update", Some(json!({"age": 31, "city": "NYC"})), 
            |current, _, update_data| {
                match (current, update_data) {
                    (Some(curr), Some(upd)) => {
                        if let (Some(curr_obj), Some(upd_obj)) = (curr.as_object(), upd.as_object()) {
                            let mut merged = curr_obj.clone();
                            for (k, v) in upd_obj {
                                merged.insert(k.clone(), v.clone());
                            }
                            Ok(json!(merged))
                        } else {
                            Ok(upd.clone())
                        }
                    },
                    _ => Ok(json!({}))
                }
            }
        ).unwrap();
        
        assert_eq!(updated, json!({"name": "Alice", "age": 31, "city": "NYC"}));
    }
    
    #[test]
    fn test_snapshot_operations() {
        let store = StateStore::new();
        
        // Create some state
        store.set("entity1", json!({"v": 1})).unwrap();
        store.set("entity2", json!({"v": 2})).unwrap();
        
        // Create snapshot
        store.create_snapshot("snap1").unwrap();
        
        // Modify state
        store.set("entity1", json!({"v": 10})).unwrap();
        store.delete("entity2").unwrap();
        
        // Verify changes
        assert_eq!(store.get("entity1").unwrap(), Some(json!({"v": 10})));
        assert_eq!(store.get("entity2").unwrap(), None);
        
        // Restore snapshot
        store.restore_snapshot("snap1").unwrap();
        
        // Verify restored state
        assert_eq!(store.get("entity1").unwrap(), Some(json!({"v": 1})));
        assert_eq!(store.get("entity2").unwrap(), Some(json!({"v": 2})));
        
        // List snapshots
        let snapshots = store.list_snapshots().unwrap();
        assert!(snapshots.contains(&"snap1".to_string()));
    }
    
    #[test]
    fn test_query_with_filter() {
        let store = StateStore::new();
        
        // Create entities
        store.set("user1", json!({"type": "user", "active": true})).unwrap();
        store.set("user2", json!({"type": "user", "active": false})).unwrap();
        store.set("admin1", json!({"type": "admin", "active": true})).unwrap();
        
        // Query all
        let all = store.query(None).unwrap();
        assert_eq!(all.len(), 3);
        
        // Query with filter
        let filter = json!({"type": "user"});
        let users = store.query(Some(&filter)).unwrap();
        assert_eq!(users.len(), 2);
        assert!(users.contains_key("user1"));
        assert!(users.contains_key("user2"));
        
        // Query with multiple filters
        let filter = json!({"type": "user", "active": true});
        let active_users = store.query(Some(&filter)).unwrap();
        assert_eq!(active_users.len(), 1);
        assert!(active_users.contains_key("user1"));
    }
}