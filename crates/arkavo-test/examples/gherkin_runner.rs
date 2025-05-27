use arkavo_test::execution::runner::TestRunner;
use arkavo_test::gherkin::parser::GherkinParser;
use arkavo_test::reporting::formats::OutputFormat;
use arkavo_test::reporting::generator::ReportGenerator;
use arkavo_test::{TestError, TestHarness};
use std::path::Path;

#[tokio::main]
async fn main() -> Result<(), TestError> {
    println!("=== Arkavo Gherkin Test Runner Demo ===\n");

    // Parse the banking app feature file
    let feature_path = Path::new("examples/banking_app.feature");
    let content = std::fs::read_to_string(feature_path)
        .map_err(|e| TestError::GherkinParse(format!("Failed to read file: {}", e)))?;

    let parser = GherkinParser::new();
    let feature = parser.parse(&content)?;

    println!("Parsed feature: {}", feature.name);
    println!("Description: {}", feature.description.join(" "));
    println!("Found {} scenarios\n", feature.scenarios.len());

    // Initialize test harness and runner
    let harness = TestHarness::new()?;
    let runner = TestRunner::new();

    // Run each scenario
    let mut all_results = Vec::new();

    for scenario in &feature.scenarios {
        println!("Running scenario: {}", scenario.name);
        println!("  Tags: {:?}", scenario.tags);

        let results = runner.run_scenario(&scenario).await?;

        for result in &results {
            println!("  Step: {} - {}", result.step_text, result.status);
            if let Some(error) = &result.error {
                println!("    Error: {}", error);
            }
        }

        all_results.extend(results);
        println!();
    }

    // Generate report
    println!("Generating test report...\n");

    let generator = ReportGenerator::new();
    let markdown_report = generator.generate(
        &all_results,
        OutputFormat::Markdown,
        Some("Arkavo Banking App Test Results".to_string()),
    )?;

    // Save report
    let report_path = Path::new("test_report.md");
    std::fs::write(report_path, markdown_report)
        .map_err(|e| TestError::Reporting(format!("Failed to write report: {}", e)))?;

    println!("Report saved to: {}", report_path.display());

    // Print summary
    let total_steps = all_results.len();
    let passed_steps = all_results.iter().filter(|r| r.status == "passed").count();
    let failed_steps = all_results.iter().filter(|r| r.status == "failed").count();
    let skipped_steps = all_results.iter().filter(|r| r.status == "skipped").count();

    println!("\nTest Summary:");
    println!("  Total steps: {}", total_steps);
    println!("  Passed: {}", passed_steps);
    println!("  Failed: {}", failed_steps);
    println!("  Skipped: {}", skipped_steps);

    if failed_steps == 0 {
        println!("\n✅ All tests passed!");
    } else {
        println!("\n❌ Some tests failed!");
    }

    Ok(())
}
