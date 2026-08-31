use arkavo_events::{Event, EventPayload};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::{DebuggerError, Result};

#[derive(Debug, Clone)]
pub struct SessionReplay {
    session_id: String,
    events: Vec<Event>,
    current_position: usize,
}

impl SessionReplay {
    pub fn new(session_id: String, events: Vec<Event>) -> Self {
        Self {
            session_id,
            events,
            current_position: 0,
        }
    }

    pub fn from_events(mut events: Vec<Event>) -> Result<Self> {
        if events.is_empty() {
            return Err(DebuggerError::Replay("No events to replay".to_string()));
        }

        // Sort by sequence number
        events.sort_by_key(|e| e.sequence);

        let session_id = events[0].session_id.clone();

        // Verify all events are from the same session
        if !events.iter().all(|e| e.session_id == session_id) {
            return Err(DebuggerError::Replay(
                "Events from multiple sessions".to_string(),
            ));
        }

        Ok(Self {
            session_id,
            events,
            current_position: 0,
        })
    }

    pub fn next_event(&mut self) -> Option<&Event> {
        if self.current_position < self.events.len() {
            let event = &self.events[self.current_position];
            self.current_position += 1;
            Some(event)
        } else {
            None
        }
    }

    pub fn previous(&mut self) -> Option<&Event> {
        if self.current_position > 1 {
            self.current_position -= 1;
            Some(&self.events[self.current_position - 1])
        } else if self.current_position == 1 {
            self.current_position = 0;
            Some(&self.events[0])
        } else {
            None
        }
    }

    pub fn jump_to(&mut self, position: usize) -> Option<&Event> {
        if position < self.events.len() {
            self.current_position = position;
            Some(&self.events[position])
        } else {
            None
        }
    }

    pub fn jump_to_time(&mut self, timestamp: DateTime<Utc>) -> Option<&Event> {
        // Find the first event at or after the timestamp
        let position = self.events.iter().position(|e| e.timestamp >= timestamp)?;
        self.jump_to(position)
    }

    pub fn reset(&mut self) {
        self.current_position = 0;
    }

    pub fn current(&self) -> Option<&Event> {
        if self.current_position > 0 && self.current_position <= self.events.len() {
            Some(&self.events[self.current_position - 1])
        } else {
            None
        }
    }

    pub fn position(&self) -> usize {
        self.current_position
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn get_summary(&self) -> SessionSummary {
        let mut summary = SessionSummary {
            session_id: self.session_id.clone(),
            total_events: self.events.len(),
            start_time: self.events.first().map(|e| e.timestamp),
            end_time: self.events.last().map(|e| e.timestamp),
            event_types: HashMap::new(),
            total_errors: 0,
            agents: HashMap::new(),
        };

        for event in &self.events {
            let event_type = event.event_type();
            *summary
                .event_types
                .entry(event_type.to_string())
                .or_insert(0) += 1;
            *summary
                .agents
                .entry(event.metadata.agent_id.clone())
                .or_insert(0) += 1;

            if matches!(&event.payload, EventPayload::Error { .. }) {
                summary.total_errors += 1;
            }
        }

        summary
    }

    pub fn filter_by_type(&self, event_type: &str) -> Vec<&Event> {
        self.events
            .iter()
            .filter(|e| e.event_type() == event_type)
            .collect()
    }

    pub fn filter_by_agent(&self, agent_id: &str) -> Vec<&Event> {
        self.events
            .iter()
            .filter(|e| e.metadata.agent_id == agent_id)
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub total_events: usize,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub event_types: HashMap<String, usize>,
    pub total_errors: usize,
    pub agents: HashMap<String, usize>,
}

impl SessionSummary {
    pub fn duration(&self) -> Option<chrono::Duration> {
        match (self.start_time, self.end_time) {
            (Some(start), Some(end)) => Some(end.signed_duration_since(start)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_events::{Event, EventMetadata, EventPayload};
    use uuid::Uuid;

    fn create_test_event(sequence: u64, event_type: &str) -> Event {
        Event {
            id: Uuid::new_v4(),
            session_id: "test-session".to_string(),
            sequence,
            timestamp: Utc::now(),
            metadata: EventMetadata {
                agent_id: "test-agent".to_string(),
                schema_version: "1.0.0".to_string(),
                parent_event_id: None,
                correlation_id: None,
                taint_chain: None,
            },
            payload: match event_type {
                "error" => EventPayload::Error {
                    error_type: "test_error".to_string(),
                    message: "Test error".to_string(),
                    stack_trace: None,
                    recoverable: Some(true),
                },
                _ => EventPayload::ReasoningStep {
                    step_type: event_type.to_string(),
                    description: "Test step".to_string(),
                    metadata: None,
                },
            },
        }
    }

    #[test]
    fn test_replay_navigation() {
        let events = vec![
            create_test_event(1, "start"),
            create_test_event(2, "middle"),
            create_test_event(3, "end"),
        ];

        let mut replay = SessionReplay::new("test-session".to_string(), events);

        assert_eq!(replay.position(), 0);
        assert_eq!(replay.len(), 3);

        // Test next
        let event1 = replay.next_event().unwrap();
        assert_eq!(event1.sequence, 1);
        assert_eq!(replay.position(), 1);

        let event2 = replay.next_event().unwrap();
        assert_eq!(event2.sequence, 2);

        // Test previous
        let prev = replay.previous().unwrap();
        assert_eq!(prev.sequence, 1);

        // Test jump
        let jumped = replay.jump_to(2).unwrap();
        assert_eq!(jumped.sequence, 3);

        // Test reset
        replay.reset();
        assert_eq!(replay.position(), 0);
    }

    #[test]
    fn test_replay_summary() {
        let events = vec![
            create_test_event(1, "start"),
            create_test_event(2, "error"),
            create_test_event(3, "end"),
        ];

        let replay = SessionReplay::new("test-session".to_string(), events);
        let summary = replay.get_summary();

        assert_eq!(summary.total_events, 3);
        assert_eq!(summary.total_errors, 1);
        assert_eq!(summary.event_types.get("error"), Some(&1));
        assert_eq!(summary.agents.get("test-agent"), Some(&3));
    }
}
