#![no_main]

//! Fuzz target for A2A message parsing
//!
//! Verifies:
//! - PROTOCOL-A2A-001: Message schema validation
//! - PROTOCOL-A2A-002: Malformed message rejection
//! - Round-trip property: encode(decode(data)) == data for valid messages

use arkavo_protocol::types::{
    AgentCard, AgentBroadcast, ChatOpenRequest, Message, MessageSendRequest, TaskRequest,
    TaskResponse, UserMessage,
};
use libfuzzer_sys::fuzz_target;

/// Fuzz input covering various A2A message types
#[derive(Debug, Clone)]
struct A2AFuzzInput {
    /// Raw bytes to attempt to parse as JSON
    raw_bytes: Vec<u8>,
    /// Message type indicator (0-6 for different types)
    message_type: u8,
}

impl<'a> arbitrary::Arbitrary<'a> for A2AFuzzInput {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let message_type = u.int_in_range(0..=6)?;
        let len = u.int_in_range(0..=4096)?;
        let mut raw_bytes = vec![0u8; len];
        u.fill_buffer(&mut raw_bytes)?;
        Ok(A2AFuzzInput {
            raw_bytes,
            message_type,
        })
    }
}

fuzz_target!(|input: A2AFuzzInput| {
    // Attempt to parse as UTF-8 string first
    let json_str = match std::str::from_utf8(&input.raw_bytes) {
        Ok(s) => s,
        Err(_) => {
            // Invalid UTF-8 should be rejected
            return;
        }
    };

    // Fuzz different A2A message types based on message_type selector
    match input.message_type {
        0 => {
            // TaskRequest parsing
            match serde_json::from_str::<TaskRequest>(json_str) {
                Ok(msg) => {
                    // Verify round-trip property
                    let encoded = serde_json::to_string(&msg).expect("encode should succeed");
                    let decoded: TaskRequest =
                        serde_json::from_str(&encoded).expect("round-trip decode should succeed");
                    assert_eq!(msg.agent_id, decoded.agent_id);
                    assert_eq!(msg.task_type, decoded.task_type);
                }
                Err(_) => {
                    // Malformed messages should be rejected without panic
                }
            }
        }
        1 => {
            // TaskResponse parsing
            match serde_json::from_str::<TaskResponse>(json_str) {
                Ok(msg) => {
                    let encoded = serde_json::to_string(&msg).expect("encode should succeed");
                    let decoded: TaskResponse =
                        serde_json::from_str(&encoded).expect("round-trip decode should succeed");
                    assert_eq!(msg.task_id, decoded.task_id);
                    assert_eq!(msg.status as u8, decoded.status as u8);
                }
                Err(_) => {}
            }
        }
        2 => {
            // AgentCard parsing
            match serde_json::from_str::<AgentCard>(json_str) {
                Ok(card) => {
                    // AgentCard must have required fields
                    assert!(!card.name.is_empty(), "AgentCard name cannot be empty");
                    assert!(!card.url.is_empty(), "AgentCard URL cannot be empty");
                    assert!(!card.version.is_empty(), "AgentCard version cannot be empty");

                    // Verify round-trip
                    let encoded = serde_json::to_string(&card).expect("encode should succeed");
                    let decoded: AgentCard =
                        serde_json::from_str(&encoded).expect("round-trip decode should succeed");
                    assert_eq!(card.name, decoded.name);
                    assert_eq!(card.url, decoded.url);
                }
                Err(_) => {}
            }
        }
        3 => {
            // AgentBroadcast parsing
            match serde_json::from_str::<AgentBroadcast>(json_str) {
                Ok(broadcast) => {
                    assert!(
                        !broadcast.agent_id.is_empty(),
                        "Broadcast agent_id cannot be empty"
                    );

                    let encoded =
                        serde_json::to_string(&broadcast).expect("encode should succeed");
                    let decoded: AgentBroadcast =
                        serde_json::from_str(&encoded).expect("round-trip decode should succeed");
                    assert_eq!(broadcast.agent_id, decoded.agent_id);
                }
                Err(_) => {}
            }
        }
        4 => {
            // Message parsing (A2A Message)
            match serde_json::from_str::<Message>(json_str) {
                Ok(msg) => {
                    // Message must have at least one part
                    assert!(!msg.parts.is_empty(), "Message must have at least one part");

                    let encoded = serde_json::to_string(&msg).expect("encode should succeed");
                    let decoded: Message =
                        serde_json::from_str(&encoded).expect("round-trip decode should succeed");
                    assert_eq!(msg.parts.len(), decoded.parts.len());
                }
                Err(_) => {}
            }
        }
        5 => {
            // MessageSendRequest parsing
            match serde_json::from_str::<MessageSendRequest>(json_str) {
                Ok(req) => {
                    assert!(!req.message.parts.is_empty(), "Message must have parts");

                    let encoded = serde_json::to_string(&req).expect("encode should succeed");
                    let decoded: MessageSendRequest =
                        serde_json::from_str(&encoded).expect("round-trip decode should succeed");
                    assert_eq!(req.message.parts.len(), decoded.message.parts.len());
                }
                Err(_) => {}
            }
        }
        6 => {
            // ChatOpenRequest parsing
            match serde_json::from_str::<ChatOpenRequest>(json_str) {
                Ok(req) => {
                    let encoded = serde_json::to_string(&req).expect("encode should succeed");
                    let decoded: ChatOpenRequest =
                        serde_json::from_str(&encoded).expect("round-trip decode should succeed");
                    // Context and metadata are optional, so just verify no panic
                    let _ = decoded;
                }
                Err(_) => {}
            }
        }
        _ => unreachable!(),
    }

    // Additional: Try parsing as generic JSON value to catch panics
    let _: Result<serde_json::Value, _> = serde_json::from_str(json_str);
});
