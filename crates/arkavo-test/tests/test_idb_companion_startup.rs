use arkavo_test::Result;
use arkavo_test::mcp::idb_wrapper::IdbWrapper;

#[tokio::test]
async fn test_idb_companion_can_start() -> Result<()> {
    // Initialize IDB wrapper which handles companion setup
    IdbWrapper::initialize()?;

    // List targets to verify companion is working
    let targets = IdbWrapper::list_targets().await?;

    // The response could be an array or an object with raw output
    let device_count = if let Some(array) = targets.as_array() {
        array.len()
    } else if let Some(raw_output) = targets.get("raw_output").and_then(|v| v.as_str()) {
        // Count newline-delimited JSON objects
        raw_output
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
    } else {
        panic!("Unexpected response format: {:?}", targets);
    };

    println!("✅ IDB companion started successfully!");
    println!("✅ Found {} devices/simulators", device_count);

    // Print some details
    if let Some(raw_output) = targets.get("raw_output").and_then(|v| v.as_str()) {
        println!("\nSample devices:");
        for (i, line) in raw_output.lines().take(3).enumerate() {
            if let Ok(device) = serde_json::from_str::<serde_json::Value>(line) {
                if let (Some(name), Some(model), Some(state)) = (
                    device.get("name").and_then(|v| v.as_str()),
                    device.get("model").and_then(|v| v.as_str()),
                    device.get("state").and_then(|v| v.as_str()),
                ) {
                    println!("  {}. {} - {} ({})", i + 1, name, model, state);
                }
            }
        }
    }

    Ok(())
}
