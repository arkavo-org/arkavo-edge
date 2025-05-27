#[cfg(test)]
mod tests {
    use super::super::mcp;
    use serde_json::{json, Value};
    use std::io::Write;
    use std::process::{Command, Stdio};
    
    fn run_mcp_request(request: Value) -> Result<Value, Box<dyn std::error::Error>> {
        let mut child = Command::new("cargo")
            .args(&["run", "--bin", "arkavo", "--", "serve"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
            
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(request.to_string().as_bytes())?;
        stdin.write_all(b"\n")?;
        drop(stdin);
        
        let output = child.wait_with_output()?;
        let response_str = String::from_utf8(output.stdout)?;
        
        for line in response_str.lines() {
            if let Ok(response) = serde_json::from_str::<Value>(line) {
                return Ok(response);
            }
        }
        
        Err("No valid JSON response found".into())
    }
    
    #[test]
    fn test_mcp_initialize_protocol_compliance() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "test",
                    "version": "1.0"
                }
            }
        });
        
        let response = run_mcp_request(request).expect("Failed to get response");
        
        // Check required fields
        assert_eq!(response["jsonrpc"], "2.0");
        assert!(response["id"].is_number() || response["id"].is_string());
        assert!(response["result"].is_object());
        
        let result = &response["result"];
        assert!(result["protocolVersion"].is_string());
        assert!(result["capabilities"].is_object());
        assert!(result["serverInfo"].is_object());
        
        let server_info = &result["serverInfo"];
        assert!(server_info["name"].is_string());
        assert!(server_info["version"].is_string());
    }
    
    #[test]
    fn test_mcp_tools_list_compliance() {
        // Would need to initialize first, then list tools
        // This is a placeholder for the full test
    }
}