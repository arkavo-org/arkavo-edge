#![no_main]

use arkavo_protocol::transport::{A2aRequest, A2aResponse};
use libfuzzer_sys::fuzz_target;
use serde_json;

fuzz_target!(|data: &[u8]| {
    // Try to parse as UTF-8
    if let Ok(s) = std::str::from_utf8(data) {
        // Try parsing as A2aRequest
        if let Ok(request) = serde_json::from_str::<A2aRequest>(s) {
            // Verify round-trip
            let serialized = serde_json::to_string(&request).unwrap();
            let reparsed: A2aRequest = serde_json::from_str(&serialized).unwrap();
            
            // Basic invariants
            assert_eq!(request.jsonrpc, reparsed.jsonrpc);
            assert_eq!(request.method, reparsed.method);
            assert_eq!(request.id, reparsed.id);
            
            // Schema round-trip validation: ensure canonical form
            let canonical = serde_json::to_string(&reparsed).unwrap();
            let recanonical = serde_json::to_string(&serde_json::from_str::<A2aRequest>(&canonical).unwrap()).unwrap();
            assert_eq!(canonical, recanonical, "Request schema round-trip failed");
            
            // Method should be one of our known methods
            match request.method.as_str() {
                "task_request" | "task_declare" | "agent_discover" | "rpc.discover" => {},
                _ => {
                    // Unknown method is still valid JSON-RPC
                }
            }
        }
        
        // Try parsing as A2aResponse
        if let Ok(response) = serde_json::from_str::<A2aResponse>(s) {
            // Verify round-trip
            let serialized = serde_json::to_string(&response).unwrap();
            let reparsed: A2aResponse = serde_json::from_str(&serialized).unwrap();
            
            // Check variant-specific fields
            match (&response, &reparsed) {
                (A2aResponse::Success { jsonrpc: j1, id: i1, .. }, 
                 A2aResponse::Success { jsonrpc: j2, id: i2, .. }) => {
                    assert_eq!(j1, j2);
                    assert_eq!(i1, i2);
                }
                (A2aResponse::Error { jsonrpc: j1, id: i1, .. }, 
                 A2aResponse::Error { jsonrpc: j2, id: i2, .. }) => {
                    assert_eq!(j1, j2);
                    assert_eq!(i1, i2);
                }
                _ => panic!("Response variant mismatch after round-trip"),
            }
            
            // Schema round-trip validation: ensure canonical form
            let canonical = serde_json::to_string(&reparsed).unwrap();
            let recanonical = serde_json::to_string(&serde_json::from_str::<A2aResponse>(&canonical).unwrap()).unwrap();
            assert_eq!(canonical, recanonical, "Response schema round-trip failed");
        }
    }
});