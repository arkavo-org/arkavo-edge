//! Deliberately fragile request parsing sample for the review tasks.

pub fn process_request(data: &[u8]) -> String {
    let parsed: serde_json::Value = serde_json::from_slice(data).unwrap();
    let id = parsed["id"].as_str().unwrap();
    let payload = parsed["payload"].as_str().unwrap();
    let decoded = base64_decode(payload).unwrap();
    format!("{}:{}", id, String::from_utf8(decoded).unwrap())
}

fn base64_decode(_input: &str) -> Result<Vec<u8>, ()> {
    Ok(vec![104, 105])
}
