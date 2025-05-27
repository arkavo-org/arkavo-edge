use arkavo_test::{TestHarness, TestError};
use arkavo_test::ai::{AnalysisEngine, CodeContext};
use arkavo_test::execution::IntelligentRunner;
use std::env;

/// Example: Testing an iOS app with Arkavo's intelligent test generation
#[tokio::main]
async fn main() -> Result<(), TestError> {
    println!("🚀 Arkavo iOS App Testing Example\n");

    // Step 1: Set up the test harness
    let harness = TestHarness::new()?;
    let mcp_server = harness.mcp_server();
    
    // Step 2: Initialize the AI analysis engine
    let api_key = env::var("ANTHROPIC_API_KEY")
        .expect("Please set ANTHROPIC_API_KEY environment variable");
    let analysis_engine = AnalysisEngine::with_api_key(api_key);
    
    // Step 3: Analyze your iOS app code
    println!("📱 Analyzing iOS app code...\n");
    
    let payment_view_controller = CodeContext {
        file_path: "PaymentViewController.swift".to_string(),
        code: r#"
class PaymentViewController: UIViewController {
    @IBOutlet weak var amountField: UITextField!
    @IBOutlet weak var cardNumberField: UITextField!
    @IBOutlet weak var payButton: UIButton!
    
    var currentUser: User?
    var paymentService = PaymentService()
    
    @IBAction func processPayment() {
        guard let amount = Double(amountField.text ?? ""),
              let cardNumber = cardNumberField.text,
              amount > 0 else {
            showError("Invalid payment details")
            return
        }
        
        guard let user = currentUser,
              user.balance >= amount else {
            showError("Insufficient funds")
            return
        }
        
        paymentService.processPayment(
            amount: amount,
            cardNumber: cardNumber,
            userId: user.id
        ) { [weak self] result in
            switch result {
            case .success(let transaction):
                self?.showSuccess(transaction)
            case .failure(let error):
                self?.showError(error.localizedDescription)
            }
        }
    }
}
"#.to_string(),
        language: "swift".to_string(),
    };
    
    // Analyze the code
    let analysis = analysis_engine.analyze_code(&payment_view_controller).await?;
    
    println!("Found {} entities:", analysis.entities.len());
    for entity in &analysis.entities {
        println!("  - {}: {:?}", entity.name, entity.attributes);
    }
    
    println!("\nFound {} critical operations:", analysis.operations.len());
    for op in &analysis.operations {
        println!("  - {}", op.name);
        for pre in &op.preconditions {
            println!("    Requires: {}", pre);
        }
    }
    
    // Step 4: Discover properties and invariants
    println!("\n🔍 Discovering properties that should always hold...\n");
    
    let properties = analysis_engine.discover_properties(&analysis).await?;
    
    for prop in &properties {
        println!("Property: {}", prop.name);
        println!("  {}", prop.description);
        println!("  Invariant: {}", prop.invariant);
        println!("  Severity: {:?}\n", prop.severity);
    }
    
    // Step 5: Generate test cases for critical properties
    println!("🧪 Generating intelligent test cases...\n");
    
    let critical_property = properties.iter()
        .find(|p| p.name.contains("payment"))
        .expect("Should find payment property");
    
    let test_cases = analysis_engine.generate_test_cases(critical_property, 10).await?;
    
    println!("Generated {} test cases for '{}'\n", test_cases.len(), critical_property.name);
    
    // Step 6: Connect to iOS app and execute tests
    println!("📲 Connecting to iOS app via MCP...\n");
    
    // Create test scenarios
    for (i, test_case) in test_cases.iter().enumerate() {
        println!("Test Case {}: {}", i + 1, test_case.description);
        println!("  Input: {}", serde_json::to_string_pretty(&test_case.inputs)?);
        
        // Execute via MCP
        let test_request = arkavo_test::mcp::server::ToolRequest {
            tool_name: "run_test".to_string(),
            params: serde_json::json!({
                "test_name": format!("payment_test_{}", i),
                "inputs": test_case.inputs,
                "timeout": 30
            }),
        };
        
        match mcp_server.call_tool(test_request).await {
            Ok(response) => {
                println!("  Result: {}", response.result);
                
                // If test failed, analyze the failure
                if let Some(failed) = response.result.get("failed").and_then(|v| v.as_bool()) {
                    if failed {
                        let error = response.result.get("error")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown error");
                        
                        let bug_analysis = analysis_engine
                            .analyze_failure(&test_case, error)
                            .await?;
                        
                        println!("  ❌ BUG FOUND!");
                        println!("     Root cause: {}", bug_analysis.root_cause);
                        println!("     Suggested fix: {}", bug_analysis.suggested_fix);
                        println!("     Minimal reproduction:");
                        println!("{}", bug_analysis.minimal_reproduction);
                    } else {
                        println!("  ✅ Test passed");
                    }
                }
            }
            Err(e) => println!("  Error: {}", e),
        }
        println!();
    }
    
    // Step 7: Run chaos testing
    println!("🌪️  Running chaos testing...\n");
    
    let chaos_scenarios = vec![
        ("Network failure during payment", "network_timeout"),
        ("App backgrounded during transaction", "app_background"),
        ("Low memory during processing", "memory_pressure"),
    ];
    
    for (scenario, failure_type) in chaos_scenarios {
        println!("Chaos scenario: {}", scenario);
        
        let chaos_request = arkavo_test::mcp::server::ToolRequest {
            tool_name: "run_test".to_string(),
            params: serde_json::json!({
                "test_name": "chaos_test",
                "chaos": {
                    "failure_type": failure_type,
                    "probability": 0.5,
                    "duration": "5s"
                },
                "timeout": 60
            }),
        };
        
        match mcp_server.call_tool(chaos_request).await {
            Ok(response) => {
                let recovered = response.result.get("recovered")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                
                if recovered {
                    println!("  ✅ App recovered gracefully");
                } else {
                    println!("  ❌ App failed to recover");
                }
            }
            Err(e) => println!("  Error: {}", e),
        }
    }
    
    println!("\n🎉 Testing complete!");
    println!("\nTo integrate with your iOS app:");
    println!("1. Add ArkavoTestBridge.framework to your test target");
    println!("2. Configure MCP server in your test environment");
    println!("3. Run 'arkavo test --explore' to find more bugs!");
    
    Ok(())
}