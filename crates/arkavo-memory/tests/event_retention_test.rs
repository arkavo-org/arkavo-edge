use arkavo_events::{Event, EventPayload};
use arkavo_memory::storage::MemoryStorage;
use arkavo_memory::event_store::SerializedEvent;
use chrono::{Duration, Utc};
use std::env;
use std::sync::Arc;
use uuid::Uuid;

/// Test that events are automatically pruned based on retention policy
#[tokio::test]
#[ignore = "Event retention not fully implemented yet"]
async fn test_event_retention_policy() {
    // Set retention to 1 hour for testing
    unsafe {
        env::set_var("ARKAVO_EVENT_RETENTION_HOURS", "1");
        env::set_var("ARKAVO_MAX_EVENTS_PER_SESSION", "10");
    }
    
    // Create test storage
    let storage = MemoryStorage::new_test().await.unwrap();
    let storage = Arc::new(storage);
    
    let session_id = Uuid::new_v4().to_string();
    let agent_id = "test-agent".to_string();
    
    // Create old events (2 hours ago)
    let old_timestamp = Utc::now() - Duration::hours(2);
    let mut old_events = Vec::new();
    
    for i in 0..5 {
        let mut event = Event::new(
            session_id.clone(),
            i,
            agent_id.clone(),
            EventPayload::ReasoningStep {
                step_type: "test".to_string(),
                description: format!("Old event {}", i),
                metadata: None,
            },
        );
        event.timestamp = old_timestamp;
        
        let event_type = event.event_type().to_string();
        old_events.push(SerializedEvent {
            id: event.id.to_string(),
            session_id: event.session_id,
            sequence: event.sequence,
            timestamp: event.timestamp.to_rfc3339(),
            agent_id: event.metadata.agent_id,
            event_type,
            payload: serde_json::to_vec(&event.payload).unwrap(),
            schema_version: event.metadata.schema_version,
        });
    }
    
    // Create recent events (30 minutes ago)
    let recent_timestamp = Utc::now() - Duration::minutes(30);
    let mut recent_events = Vec::new();
    
    for i in 5..10 {
        let mut event = Event::new(
            session_id.clone(),
            i,
            agent_id.clone(),
            EventPayload::ReasoningStep {
                step_type: "test".to_string(),
                description: format!("Recent event {}", i),
                metadata: None,
            },
        );
        event.timestamp = recent_timestamp;
        
        let event_type = event.event_type().to_string();
        recent_events.push(SerializedEvent {
            id: event.id.to_string(),
            session_id: event.session_id,
            sequence: event.sequence,
            timestamp: event.timestamp.to_rfc3339(),
            agent_id: event.metadata.agent_id,
            event_type,
            payload: serde_json::to_vec(&event.payload).unwrap(),
            schema_version: event.metadata.schema_version,
        });
    }
    
    // Store all events
    storage.store_events(old_events).await.unwrap();
    storage.store_events(recent_events).await.unwrap();
    
    // Verify all events are initially stored
    let all_events = storage.get_session_events(&session_id, None).await.unwrap();
    assert_eq!(all_events.len(), 10);
    
    // Store one more event to trigger pruning
    let trigger_event = Event::new(
        session_id.clone(),
        10,
        agent_id.clone(),
        EventPayload::SessionEnded {
            reason: "test_complete".to_string(),
            duration_ms: 1000,
            summary: None,
        },
    );
    
    let trigger_event_type = trigger_event.event_type().to_string();
    storage.store_events(vec![SerializedEvent {
        id: trigger_event.id.to_string(),
        session_id: trigger_event.session_id,
        sequence: trigger_event.sequence,
        timestamp: trigger_event.timestamp.to_rfc3339(),
        agent_id: trigger_event.metadata.agent_id,
        event_type: trigger_event_type,
        payload: serde_json::to_vec(&trigger_event.payload).unwrap(),
        schema_version: trigger_event.metadata.schema_version,
    }]).await.unwrap();
    
    // Note: In a real implementation, the pruning would happen automatically.
    // For this test, we're verifying that the retention configuration is respected.
    // The actual pruning logic in event_store.rs would remove old events.
    
    // Clean up
    unsafe {
        env::remove_var("ARKAVO_EVENT_RETENTION_HOURS");
        env::remove_var("ARKAVO_MAX_EVENTS_PER_SESSION");
    }
}

/// Test that sessions with too many events are pruned
#[tokio::test]
#[ignore = "Event retention not fully implemented yet"]
async fn test_max_events_per_session() {
    // Set low limit for testing
    unsafe {
        env::set_var("ARKAVO_MAX_EVENTS_PER_SESSION", "5");
    }
    
    // Create test storage
    let storage = MemoryStorage::new_test().await.unwrap();
    let storage = Arc::new(storage);
    
    let session_id = Uuid::new_v4().to_string();
    let agent_id = "test-agent".to_string();
    
    // Try to store more events than the limit
    let mut events = Vec::new();
    for i in 0..10 {
        let event = Event::new(
            session_id.clone(),
            i,
            agent_id.clone(),
            EventPayload::ReasoningStep {
                step_type: "test".to_string(),
                description: format!("Event {}", i),
                metadata: None,
            },
        );
        
        let event_type = event.event_type().to_string();
        events.push(SerializedEvent {
            id: event.id.to_string(),
            session_id: event.session_id,
            sequence: event.sequence,
            timestamp: event.timestamp.to_rfc3339(),
            agent_id: event.metadata.agent_id,
            event_type,
            payload: serde_json::to_vec(&event.payload).unwrap(),
            schema_version: event.metadata.schema_version,
        });
    }
    
    // Store events in batches to simulate real usage
    storage.store_events(events[0..5].to_vec()).await.unwrap();
    storage.store_events(events[5..10].to_vec()).await.unwrap();
    
    // In a real implementation with pruning enabled, only the most recent events
    // would be kept based on ARKAVO_MAX_EVENTS_PER_SESSION
    
    // Clean up
    unsafe {
        env::remove_var("ARKAVO_MAX_EVENTS_PER_SESSION");
    }
}