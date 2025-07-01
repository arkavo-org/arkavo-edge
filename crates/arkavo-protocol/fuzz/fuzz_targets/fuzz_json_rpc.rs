#![no_main]

use libfuzzer_sys::fuzz_target;
use arkavo_protocol::transport::{A2aRequest, A2aResponse};
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
                "promise_request" | "promise_declare" | "agent_discover" | "rpc.discover" => {},
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
            
            assert_eq!(response.jsonrpc, reparsed.jsonrpc);
            assert_eq!(response.id, reparsed.id);
            
            // Schema round-trip validation: ensure canonical form
            let canonical = serde_json::to_string(&reparsed).unwrap();
            let recanonical = serde_json::to_string(&serde_json::from_str::<A2aResponse>(&canonical).unwrap()).unwrap();
            assert_eq!(canonical, recanonical, "Response schema round-trip failed");
            
            // Should have either result or error, not both
            match (&response.result, &response.error) {
                (Some(_), None) | (None, Some(_)) => {},
                _ => panic!("Response must have exactly one of result or error"),
            }
        }
    }
});