use arkavo_gemini::RestClient;
use serde_json::json;
use std::fs;
use std::process::Command;
use std::time::Instant;

#[derive(Debug)]
struct QualityTestResult {
    test_name: String,
    passed: bool,
    duration_ms: u64,
    score: f64,
    details: String,
}

fn get_api_key() -> String {
    std::env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY must be set")
}

fn get_model() -> String {
    std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "models/gemini-3-pro-preview".to_string())
}

async fn test_code_compilation(client: &RestClient) -> QualityTestResult {
    let start = Instant::now();
    let prompt = "Write a simple Rust function that calculates the factorial of a number. Only output the function code, no explanation.";

    let mut stream = client
        .stream_generate_content(prompt, None)
        .await
        .expect("Failed to create stream");

    let mut code = String::new();

    while let Some(result) = stream.next().await {
        if let Ok(chunk) = result {
            if let Some(text) = chunk.text {
                code.push_str(&text);
            }
        }
    }

    let code = code
        .lines()
        .filter(|line| !line.trim().starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n");

    let temp_file = "/tmp/gemini_test_factorial.rs";
    let test_code = format!(
        r#"
{}

fn main() {{
    println!("{{:?}}", factorial(5));
}}
"#,
        code
    );

    fs::write(temp_file, test_code).expect("Failed to write test file");

    let compile_result = Command::new("rustc")
        .arg(temp_file)
        .arg("-o")
        .arg("/tmp/gemini_test_factorial")
        .output();

    let duration = start.elapsed();
    let passed = compile_result
        .as_ref()
        .map(|output| output.status.success())
        .unwrap_or(false);

    let score = if passed { 1.0 } else { 0.0 };

    let details = if passed {
        "Code compiled successfully".to_string()
    } else {
        format!(
            "Compilation failed: {}",
            String::from_utf8_lossy(
                &compile_result
                    .unwrap_or_else(|e| panic!("Failed to run rustc: {}", e))
                    .stderr
            )
        )
    };

    QualityTestResult {
        test_name: "Code Compilation".to_string(),
        passed,
        duration_ms: duration.as_millis() as u64,
        score,
        details,
    }
}

async fn test_needle_in_haystack(client: &RestClient) -> QualityTestResult {
    let start = Instant::now();
    let uuid = "abc123-def456-ghi789";
    let haystack = "Lorem ipsum dolor sit amet. ".repeat(1000);
    let needle_position = haystack.len() / 2;

    let text_with_needle = format!(
        "{}{} {}",
        &haystack[..needle_position],
        uuid,
        &haystack[needle_position..]
    );

    let prompt = format!(
        "{}\n\nFind the UUID in the text above. It's in the format xxx-yyy-zzz. What is it?",
        text_with_needle
    );

    let mut stream = client
        .stream_generate_content(&prompt, None)
        .await
        .expect("Failed to create stream");

    let mut response = String::new();

    while let Some(result) = stream.next().await {
        if let Ok(chunk) = result {
            if let Some(text) = chunk.text {
                response.push_str(&text);
            }
        }
    }

    let duration = start.elapsed();
    let passed = response.contains(uuid);
    let score = if passed { 1.0 } else { 0.0 };

    let details = if passed {
        "Successfully found UUID in large context".to_string()
    } else {
        format!("Failed to find UUID. Response: {}", response)
    };

    QualityTestResult {
        test_name: "Needle in Haystack".to_string(),
        passed,
        duration_ms: duration.as_millis() as u64,
        score,
        details,
    }
}

async fn test_factual_accuracy(client: &RestClient) -> QualityTestResult {
    let start = Instant::now();

    let questions = vec![
        ("What is the capital of France?", "Paris"),
        ("What is 15 * 8?", "120"),
        ("Who wrote Romeo and Juliet?", "Shakespeare"),
    ];

    let mut correct_count = 0;
    let mut details_vec = Vec::new();

    for (question, expected) in questions.iter() {
        let mut stream = client
            .stream_generate_content(*question, None)
            .await
            .expect("Failed to create stream");

        let mut response = String::new();

        while let Some(result) = stream.next().await {
            if let Ok(chunk) = result {
                if let Some(text) = chunk.text {
                    response.push_str(&text);
                }
            }
        }

        let response_lower = response.to_lowercase();
        let expected_lower = expected.to_lowercase();

        if response_lower.contains(&expected_lower) {
            correct_count += 1;
            details_vec.push(format!("✓ {}: {}", question, expected));
        } else {
            details_vec.push(format!(
                "✗ {}: Expected {}, got {}",
                question, expected, response
            ));
        }
    }

    let duration = start.elapsed();
    let score = correct_count as f64 / questions.len() as f64;
    let passed = score >= 0.67;

    let details = details_vec.join("\n");

    QualityTestResult {
        test_name: "Factual Accuracy".to_string(),
        passed,
        duration_ms: duration.as_millis() as u64,
        score,
        details,
    }
}

async fn test_json_schema_compliance(client: &RestClient) -> QualityTestResult {
    let start = Instant::now();

    let prompt = r#"Generate a JSON object matching this exact schema:
{
  "name": string,
  "age": number,
  "is_active": boolean,
  "tags": array of strings
}
Only output valid JSON, nothing else."#;

    let mut stream = client
        .stream_generate_content(prompt, None)
        .await
        .expect("Failed to create stream");

    let mut response = String::new();

    while let Some(result) = stream.next().await {
        if let Ok(chunk) = result {
            if let Some(text) = chunk.text {
                response.push_str(&text);
            }
        }
    }

    let duration = start.elapsed();

    let json_result: Result<serde_json::Value, _> = serde_json::from_str(response.trim());

    let (passed, score, details) = match json_result {
        Ok(json) => {
            let has_name = json.get("name").and_then(|v| v.as_str()).is_some();
            let has_age = json.get("age").and_then(|v| v.as_u64()).is_some();
            let has_is_active = json.get("is_active").and_then(|v| v.as_bool()).is_some();
            let has_tags = json.get("tags").and_then(|v| v.as_array()).is_some();

            let field_count = [has_name, has_age, has_is_active, has_tags]
                .iter()
                .filter(|&&x| x)
                .count();
            let score = field_count as f64 / 4.0;

            (
                field_count >= 3,
                score,
                format!(
                    "Valid JSON with {}/4 required fields: name={}, age={}, is_active={}, tags={}",
                    field_count, has_name, has_age, has_is_active, has_tags
                ),
            )
        }
        Err(e) => (false, 0.0, format!("Invalid JSON: {}", e)),
    };

    QualityTestResult {
        test_name: "JSON Schema Compliance".to_string(),
        passed,
        duration_ms: duration.as_millis() as u64,
        score,
        details,
    }
}

async fn test_code_explanation_quality(client: &RestClient) -> QualityTestResult {
    let start = Instant::now();

    let code = r#"
fn quicksort<T: Ord>(arr: &mut [T]) {
    if arr.len() <= 1 { return; }
    let pivot = partition(arr);
    let (left, right) = arr.split_at_mut(pivot);
    quicksort(left);
    quicksort(&mut right[1..]);
}
"#;

    let prompt = format!("Explain this Rust code:\n{}", code);

    let mut stream = client
        .stream_generate_content(prompt, None)
        .await
        .expect("Failed to create stream");

    let mut response = String::new();

    while let Some(result) = stream.next().await {
        if let Ok(chunk) = result {
            if let Some(text) = chunk.text {
                response.push_str(&text);
            }
        }
    }

    let duration = start.elapsed();

    let response_lower = response.to_lowercase();
    let quality_indicators = [
        "quicksort",
        "algorithm",
        "sort",
        "pivot",
        "partition",
        "recursive",
    ];

    let indicator_count = quality_indicators
        .iter()
        .filter(|&term| response_lower.contains(term))
        .count();

    let score = indicator_count as f64 / quality_indicators.len() as f64;
    let passed = score >= 0.5 && response.len() > 100;

    let details = format!(
        "Explanation quality: {}/{} key terms, {} chars",
        indicator_count,
        quality_indicators.len(),
        response.len()
    );

    QualityTestResult {
        test_name: "Code Explanation Quality".to_string(),
        passed,
        duration_ms: duration.as_millis() as u64,
        score,
        details,
    }
}

#[tokio::main]
async fn main() {
    let api_key = get_api_key();
    let model = get_model();

    println!("🧪 Gemini 3 Pro Preview Quality Test Suite");
    println!("Model: {}", model);
    println!("{}", "=".repeat(60));

    let client = RestClient::new(&api_key, &model);

    let mut results = Vec::new();

    println!("\n🔨 Running Code Compilation Test...");
    results.push(test_code_compilation(&client).await);

    println!("\n🔍 Running Needle in Haystack Test...");
    results.push(test_needle_in_haystack(&client).await);

    println!("\n📚 Running Factual Accuracy Test...");
    results.push(test_factual_accuracy(&client).await);

    println!("\n📋 Running JSON Schema Compliance Test...");
    results.push(test_json_schema_compliance(&client).await);

    println!("\n💡 Running Code Explanation Quality Test...");
    results.push(test_code_explanation_quality(&client).await);

    println!("\n\n");
    println!("{}", "=".repeat(60));
    println!("📊 QUALITY TEST RESULTS");
    println!("{}", "=".repeat(60));

    let mut passed_count = 0;
    let mut total_score = 0.0;

    for result in &results {
        let status = if result.passed {
            "✅ PASS"
        } else {
            "❌ FAIL"
        };
        println!("\n{} {}", status, result.test_name);
        println!("  Duration: {}ms", result.duration_ms);
        println!("  Score: {:.1}%", result.score * 100.0);
        println!("  Details: {}", result.details);

        if result.passed {
            passed_count += 1;
        }
        total_score += result.score;
    }

    let avg_score = total_score / results.len() as f64;

    println!("\n");
    println!("{}", "=".repeat(60));
    println!("📈 SUMMARY");
    println!("{}", "=".repeat(60));
    println!("Tests Passed: {}/{}", passed_count, results.len());
    println!("Average Score: {:.1}%", avg_score * 100.0);
    println!(
        "Overall Quality: {}",
        if avg_score >= 0.8 {
            "🌟 Excellent"
        } else if avg_score >= 0.6 {
            "👍 Good"
        } else {
            "⚠️  Needs Improvement"
        }
    );

    let json_results = json!({
        "model": model,
        "tests": results.iter().map(|r| {
            json!({
                "name": r.test_name,
                "passed": r.passed,
                "duration_ms": r.duration_ms,
                "score": r.score,
                "details": r.details
            })
        }).collect::<Vec<_>>(),
        "summary": {
            "passed_count": passed_count,
            "total_tests": results.len(),
            "average_score": avg_score,
            "rating": if avg_score >= 0.8 { "excellent" } else if avg_score >= 0.6 { "good" } else { "needs_improvement" }
        }
    });

    let output_path = "/tmp/gemini-3-quality-results.json";
    fs::write(
        output_path,
        serde_json::to_string_pretty(&json_results).unwrap(),
    )
    .expect("Failed to write results");

    println!("\n📁 Results saved to: {}", output_path);
}
