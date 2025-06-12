#[cfg(target_os = "macos")]
mod tests {
    use arkavo_test::mcp::idb_tap_enhanced::IdbTapEnhanced;
    use arkavo_test::mcp::idb_companion_health::IdbCompanionHealth;
    use arkavo_test::mcp::simulator_state_verifier::SimulatorStateVerifier;
    
    #[tokio::test]
    async fn test_enhanced_tap_with_metrics() {
        // This test requires a running simulator
        let device_id = "4A05B20A-349D-4EC5-B796-8F384798268B"; // Replace with actual device ID
        
        // First verify simulator state
        match SimulatorStateVerifier::verify_ready_for_interaction(device_id, None).await {
            Ok(state) => {
                println!("Simulator state: {:?}", state);
                
                if !state.is_booted {
                    println!("Simulator not booted, skipping test");
                    return;
                }
            }
            Err(e) => {
                println!("Could not verify simulator state: {}, skipping test", e);
                return;
            }
        }
        
        // Test tap at center of screen
        let result = IdbTapEnhanced::tap_with_verification(
            device_id,
            200.0,
            400.0,
            3,
        ).await;
        
        match result {
            Ok(tap_result) => {
                println!("Tap succeeded: {}", serde_json::to_string_pretty(&tap_result).unwrap());
                
                // Check metrics
                if let Some(metrics) = IdbCompanionHealth::get_metrics(device_id) {
                    println!("Tap metrics:");
                    println!("  Success rate: {:.1}%", 
                        (metrics.total_taps_succeeded as f64 / metrics.total_taps_attempted as f64) * 100.0);
                    println!("  Average latency: {:.1}ms", metrics.average_tap_latency_ms);
                    println!("  Consecutive failures: {}", metrics.consecutive_failures);
                }
            }
            Err(e) => {
                println!("Tap failed: {}", e);
                
                // Print health report
                let health_report = IdbCompanionHealth::get_health_report();
                println!("Health report:\n{}", serde_json::to_string_pretty(&health_report).unwrap());
            }
        }
    }
    
    #[tokio::test]
    async fn test_tap_sequence_with_verification() {
        let device_id = "4A05B20A-349D-4EC5-B796-8F384798268B"; // Replace with actual device ID
        
        // Prepare simulator
        if let Err(e) = SimulatorStateVerifier::prepare_for_interaction(device_id, None).await {
            println!("Could not prepare simulator: {}, skipping test", e);
            return;
        }
        
        // Test sequence of taps
        let tap_points = vec![
            (100.0, 100.0),
            (200.0, 200.0),
            (300.0, 300.0),
        ];
        
        let results = IdbTapEnhanced::tap_sequence_with_verification(
            device_id,
            tap_points,
            500, // 500ms between taps
        ).await;
        
        match results {
            Ok(tap_results) => {
                println!("Tap sequence completed successfully:");
                for (i, result) in tap_results.iter().enumerate() {
                    println!("  Tap {}: {}", i + 1, serde_json::to_string(&result).unwrap());
                }
            }
            Err(e) => {
                println!("Tap sequence failed: {}", e);
            }
        }
        
        // Print final metrics
        if let Some(metrics) = IdbCompanionHealth::get_metrics(device_id) {
            println!("\nFinal metrics:");
            println!("  Total attempts: {}", metrics.total_taps_attempted);
            println!("  Total succeeded: {}", metrics.total_taps_succeeded);
            println!("  Success rate: {:.1}%", 
                (metrics.total_taps_succeeded as f64 / metrics.total_taps_attempted as f64) * 100.0);
            println!("  Average latency: {:.1}ms", metrics.average_tap_latency_ms);
        }
    }
}