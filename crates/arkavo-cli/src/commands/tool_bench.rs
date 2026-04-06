use clap::Args;
use serde_json::{Value, json};
use std::time::Instant;

use arkavo_llm::mcp_converter::LocalToolFormat;
use arkavo_llm::tool_parser::ToolParser;
use arkavo_mcp_tools::registry::ToolInfo;

#[derive(Args)]
pub struct ToolBenchCommand {
    /// Model name to test (e.g., "qwen3-0.6b", "ministral-3b")
    #[arg(long)]
    pub model: Option<String>,

    /// Test all available models
    #[arg(long)]
    pub all: bool,

    /// Tool call format to test
    #[arg(long, default_value = "fence")]
    pub format: String,

    /// Number of iterations per scenario
    #[arg(long, default_value = "1")]
    pub iterations: usize,

    /// Save results to JSON file
    #[arg(long)]
    pub output: Option<String>,

    /// Run tool-loop benchmark: measures full round-trip (call → result → response)
    #[arg(long)]
    pub tool_loop: bool,
}

/// A standardized tool calling scenario for benchmarking.
struct Scenario {
    name: &'static str,
    prompt: &'static str,
    expected_tool: Option<&'static str>,
    expected_params: Vec<(&'static str, ParamCheck)>,
}

enum ParamCheck {
    Present,
    IsType(&'static str),
}

fn test_tools() -> Vec<ToolInfo> {
    vec![
        ToolInfo {
            name: "get_weather".to_string(),
            category: "utility".to_string(),
            description: "Get the current weather for a location".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "location": {"type": "string", "description": "City name"},
                    "unit": {"type": "string", "enum": ["celsius", "fahrenheit"], "default": "celsius"}
                },
                "required": ["location"]
            }),
        },
        ToolInfo {
            name: "read_file".to_string(),
            category: "filesystem".to_string(),
            description: "Read the contents of a file at a given path".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "file_path": {"type": "string", "description": "Absolute path to file"},
                    "limit": {"type": "integer", "description": "Max lines to read"}
                },
                "required": ["file_path"]
            }),
        },
        ToolInfo {
            name: "search".to_string(),
            category: "search".to_string(),
            description: "Search for content matching a query string".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Search query"},
                    "limit": {"type": "integer", "description": "Max results"},
                    "case_sensitive": {"type": "boolean", "default": false}
                },
                "required": ["query"]
            }),
        },
        ToolInfo {
            name: "get_time".to_string(),
            category: "utility".to_string(),
            description: "Get the current date and time".to_string(),
            schema: json!({
                "type": "object",
                "properties": {},
            }),
        },
        ToolInfo {
            name: "run_command".to_string(),
            category: "system".to_string(),
            description: "Execute a shell command and return the output".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "Shell command to execute"},
                    "timeout_ms": {"type": "integer", "description": "Timeout in milliseconds"}
                },
                "required": ["command"]
            }),
        },
    ]
}

fn test_scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "simple_single_param",
            prompt: "What's the weather in Tokyo?",
            expected_tool: Some("get_weather"),
            expected_params: vec![("location", ParamCheck::Present)],
        },
        Scenario {
            name: "multi_param",
            prompt: "Search for 'rust async' with a limit of 5 results",
            expected_tool: Some("search"),
            expected_params: vec![
                ("query", ParamCheck::Present),
                ("limit", ParamCheck::IsType("number")),
            ],
        },
        Scenario {
            name: "no_params",
            prompt: "What time is it right now?",
            expected_tool: Some("get_time"),
            expected_params: vec![],
        },
        Scenario {
            name: "enum_param",
            prompt: "Get weather in Berlin in fahrenheit",
            expected_tool: Some("get_weather"),
            expected_params: vec![
                ("location", ParamCheck::Present),
                ("unit", ParamCheck::Present),
            ],
        },
        Scenario {
            name: "file_path",
            prompt: "Read the file at /etc/hosts",
            expected_tool: Some("read_file"),
            expected_params: vec![("file_path", ParamCheck::Present)],
        },
        Scenario {
            name: "command_execution",
            prompt: "Run the command 'ls -la /tmp'",
            expected_tool: Some("run_command"),
            expected_params: vec![("command", ParamCheck::Present)],
        },
        Scenario {
            name: "should_not_call",
            prompt: "Hello, how are you doing today?",
            expected_tool: None,
            expected_params: vec![],
        },
        Scenario {
            name: "multi_param_types",
            prompt: "Search for 'error handling' case-sensitively, return at most 10 results",
            expected_tool: Some("search"),
            expected_params: vec![
                ("query", ParamCheck::Present),
                ("limit", ParamCheck::IsType("number")),
                ("case_sensitive", ParamCheck::IsType("boolean")),
            ],
        },
    ]
}

#[derive(serde::Serialize)]
#[allow(clippy::struct_excessive_bools)]
struct ScenarioResult {
    scenario: String,
    parsed: bool,
    correct_tool: bool,
    params_present: bool,
    params_correct_type: bool,
    latency_ms: u64,
    raw_output: String,
    parsed_tool: Option<String>,
    parsed_args: Option<Value>,
}

#[derive(serde::Serialize)]
struct ModelReport {
    model: String,
    format: String,
    scenarios_total: usize,
    parse_success: usize,
    tool_name_correct: usize,
    params_present: usize,
    params_type_correct: usize,
    avg_latency_ms: f64,
    results: Vec<ScenarioResult>,
}

/// Run the tool bench without a model — just test prompt generation and parsing.
///
/// This validates the prompt → parse pipeline offline using synthetic model outputs.
pub async fn run(command: &ToolBenchCommand) -> Result<(), Box<dyn std::error::Error>> {
    if command.tool_loop {
        return run_tool_loop_bench(command).await;
    }

    let tools = test_tools();
    let scenarios = test_scenarios();
    let format = match command.format.as_str() {
        "xml" => LocalToolFormat::Xml,
        "json" => LocalToolFormat::Json,
        _ => LocalToolFormat::Fence,
    };

    println!("Tool Bench — Format: {format:?}");
    println!("═══════════════════════════════════════════════════════════════");

    // Show generated prompt for inspection
    let prompt = arkavo_llm::McpConverter::to_local_prompt(&tools, format);
    println!("\nGenerated tool prompt ({} chars):", prompt.len());
    println!("───────────────────────────────────────────────────────────────");
    for line in prompt.lines().take(30) {
        println!("  {line}");
    }
    if prompt.lines().count() > 30 {
        println!("  ... ({} more lines)", prompt.lines().count() - 30);
    }

    // Test distilled prompts at each detail level
    println!("\n───────────────────────────────────────────────────────────────");
    println!("Distilled prompt sizes:");
    for level in [
        arkavo_mcp_tools::DetailLevel::NameOnly,
        arkavo_mcp_tools::DetailLevel::NameAndDescription,
        arkavo_mcp_tools::DetailLevel::FullSchema,
    ] {
        let distilled = arkavo_llm::McpConverter::to_fence_prompt_distilled(&tools, level);
        println!("  {level:?}: {} chars", distilled.len());
    }

    // Test GBNF grammar generation
    let (grammar, root) = arkavo_llm::tool_grammar::fence_grammar_for_tools(&tools);
    println!("\nGBNF grammar ({} chars, root={root}):", grammar.len());
    for line in grammar.lines() {
        println!("  {line}");
    }

    // Test parsing synthetic model outputs
    println!("\n───────────────────────────────────────────────────────────────");
    println!("Parsing synthetic outputs:");
    println!(
        "{:<25} {:<10} {:<12} {:<12}",
        "Scenario", "Parsed", "Tool OK", "Params OK"
    );
    println!("{}", "─".repeat(60));

    let synthetic_outputs = synthetic_fence_outputs();
    let mut total_parsed = 0;
    let mut total_tool_ok = 0;
    let mut total_params_ok = 0;

    for (scenario, output) in scenarios.iter().zip(synthetic_outputs.iter()) {
        let parsed = ToolParser::parse_fence(output).unwrap_or_default();
        let registered: std::collections::HashSet<&str> =
            tools.iter().map(|t| t.name.as_str()).collect();
        let filtered: Vec<_> = parsed
            .into_iter()
            .filter(|c| registered.contains(c.tool_name.as_str()))
            .collect();

        let is_parsed = !filtered.is_empty() || scenario.expected_tool.is_none();
        let tool_ok = match scenario.expected_tool {
            Some(expected) => filtered.first().is_some_and(|c| c.tool_name == expected),
            None => filtered.is_empty(),
        };
        let params_ok = if let Some(call) = filtered.first() {
            scenario.expected_params.iter().all(|(name, check)| {
                let has_param = call.arguments.get(*name).is_some();
                match check {
                    ParamCheck::Present => has_param,
                    ParamCheck::IsType(t) => {
                        has_param
                            && match *t {
                                "number" => call.arguments[*name].is_number(),
                                "boolean" => call.arguments[*name].is_boolean(),
                                "string" => call.arguments[*name].is_string(),
                                _ => true,
                            }
                    }
                }
            })
        } else {
            scenario.expected_tool.is_none()
        };

        if is_parsed {
            total_parsed += 1;
        }
        if tool_ok {
            total_tool_ok += 1;
        }
        if params_ok {
            total_params_ok += 1;
        }

        println!(
            "{:<25} {:<10} {:<12} {:<12}",
            scenario.name,
            if is_parsed { "✓" } else { "✗" },
            if tool_ok { "✓" } else { "✗" },
            if params_ok { "✓" } else { "✗" },
        );
    }

    let total = scenarios.len();
    println!("{}", "─".repeat(60));
    println!(
        "Totals: Parse {total_parsed}/{total} | Tool {total_tool_ok}/{total} | Params {total_params_ok}/{total}"
    );

    // Determine which models to bench
    let models: Vec<String> = if command.all {
        discover_cached_models()
    } else if let Some(ref model_name) = command.model {
        vec![model_name.clone()]
    } else {
        vec![]
    };

    let mut all_reports = Vec::new();
    for model_name in &models {
        println!("\n═══════════════════════════════════════════════════════════════");
        println!("Live inference with model: {model_name}");
        println!("═══════════════════════════════════════════════════════════════");

        match run_live_bench(model_name, &tools, &scenarios, format, command.iterations).await {
            Ok(report) => {
                println!(
                    "\nResults: Parse {}/{}  Tool {}/{}  Params {}/{}  Avg latency: {:.0}ms",
                    report.parse_success,
                    report.scenarios_total,
                    report.tool_name_correct,
                    report.scenarios_total,
                    report.params_present,
                    report.scenarios_total,
                    report.avg_latency_ms,
                );
                all_reports.push(report);
            }
            Err(e) => {
                eprintln!("Live bench failed for {model_name}: {e}");
            }
        }
    }

    if models.len() > 1 {
        println!("\n═══════════════════════════════════════════════════════════════");
        println!("Summary");
        println!("═══════════════════════════════════════════════════════════════");
        println!(
            "{:<25} {:<10} {:<10} {:<10} {:<10}",
            "Model", "Parse", "Tool", "Params", "Avg ms"
        );
        println!("{}", "─".repeat(65));
        for r in &all_reports {
            println!(
                "{:<25} {}/{:<7} {}/{:<7} {}/{:<7} {:.0}",
                r.model,
                r.parse_success,
                r.scenarios_total,
                r.tool_name_correct,
                r.scenarios_total,
                r.params_present,
                r.scenarios_total,
                r.avg_latency_ms,
            );
        }
    }

    if let Some(ref path) = command.output {
        let json = if all_reports.len() == 1 {
            serde_json::to_string_pretty(&all_reports[0])?
        } else {
            serde_json::to_string_pretty(&all_reports)?
        };
        std::fs::write(path, json)?;
        println!("Results saved to {path}");
    }

    Ok(())
}

async fn run_live_bench(
    model_name: &str,
    tools: &[ToolInfo],
    scenarios: &[Scenario],
    format: LocalToolFormat,
    iterations: usize,
) -> Result<ModelReport, Box<dyn std::error::Error>> {
    use arkavo_llm::llamacpp_provider::{LlamaCppProvider, SamplingConfig};

    let config = SamplingConfig {
        tool_format: format,
        ..SamplingConfig::default()
    };

    let registry = arkavo_llm::ModelRegistry::new();

    // Try to find the model path from HuggingFace cache
    let model_path = find_model_path(model_name)?;
    registry
        .load(model_name, &model_path)
        .map_err(|e| format!("Failed to load model '{model_name}': {e}"))?;

    let provider = LlamaCppProvider::new_with_registry(
        std::sync::Arc::new(registry),
        model_name.to_string(),
        config,
    )?;

    let tools_json = arkavo_llm::McpConverter::to_anthropic_format_minimal(
        &tools
            .iter()
            .map(|t| arkavo_mcp_tools::registry::MinimalToolInfo {
                name: t.name.clone(),
                category: Some(t.category.clone()),
                description: Some(t.description.clone()),
                schema: Some(t.schema.clone()),
                aliases: None,
            })
            .collect::<Vec<_>>(),
    );

    let registered: std::collections::HashSet<&str> =
        tools.iter().map(|t| t.name.as_str()).collect();

    let mut results = Vec::new();
    let mut total_latency = 0u64;

    for scenario in scenarios {
        for _ in 0..iterations {
            let messages = vec![arkavo_llm::Message::user(scenario.prompt.to_string())];

            let start = Instant::now();
            let response = provider
                .complete_with_tools(messages, Some(tools_json.clone()), None)
                .await;
            let latency = start.elapsed().as_millis() as u64;
            total_latency += latency;

            let (parsed, correct_tool, params_present, params_type_correct, raw, ptool, pargs) =
                match response {
                    Ok(resp) => {
                        // Apply the same post-processing as the production router:
                        // 1. Filter language fences (e.g., ```python\ntool(...)```)
                        let mut calls =
                            arkavo_router::tool_extraction::filter_and_extract_tool_calls(
                                resp.tool_calls,
                            );
                        // 2. If no tool calls parsed, try text extraction fallbacks
                        //    (curly-brace format, Python-style, XML, JSON)
                        if calls.is_empty() && !resp.content.is_empty() {
                            calls = arkavo_router::tool_extraction::extract_tool_calls_from_text(
                                &resp.content,
                            );
                        }
                        // 3. Filter to registered tool names only
                        let calls: Vec<_> = calls
                            .into_iter()
                            .filter(|c| registered.contains(c.tool_name.as_str()))
                            .collect();

                        let parsed = !calls.is_empty() || scenario.expected_tool.is_none();
                        let correct_tool = match scenario.expected_tool {
                            Some(exp) => calls.first().is_some_and(|c| c.tool_name == exp),
                            None => calls.is_empty(),
                        };

                        let (pp, ptc) = if let Some(call) = calls.first() {
                            let pp = scenario
                                .expected_params
                                .iter()
                                .all(|(n, _)| call.arguments.get(*n).is_some());
                            let ptc =
                                scenario
                                    .expected_params
                                    .iter()
                                    .all(|(n, check)| match check {
                                        ParamCheck::Present => call.arguments.get(*n).is_some(),
                                        ParamCheck::IsType(t) => {
                                            call.arguments.get(*n).is_some_and(|v| match *t {
                                                "number" => v.is_number(),
                                                "boolean" => v.is_boolean(),
                                                "string" => v.is_string(),
                                                _ => true,
                                            })
                                        }
                                    });
                            (pp, ptc)
                        } else {
                            (
                                scenario.expected_tool.is_none(),
                                scenario.expected_tool.is_none(),
                            )
                        };

                        let ptool = calls.first().map(|c| c.tool_name.clone());
                        let pargs = calls.first().map(|c| c.arguments.clone());
                        // Strip think blocks for cleaner output reporting
                        let clean_content =
                            arkavo_router::response::strip_think_blocks(&resp.content);
                        (parsed, correct_tool, pp, ptc, clean_content, ptool, pargs)
                    }
                    Err(e) => (
                        false,
                        false,
                        false,
                        false,
                        format!("ERROR: {e}"),
                        None,
                        None,
                    ),
                };

            let status = if correct_tool && params_present {
                "✓"
            } else {
                "✗"
            };
            println!(
                "  {status} {:<25} tool={:<15} latency={latency}ms",
                scenario.name,
                ptool.as_deref().unwrap_or("-"),
            );

            results.push(ScenarioResult {
                scenario: scenario.name.to_string(),
                parsed,
                correct_tool,
                params_present,
                params_correct_type: params_type_correct,
                latency_ms: latency,
                raw_output: raw,
                parsed_tool: ptool,
                parsed_args: pargs,
            });
        }
    }

    let n = results.len();
    Ok(ModelReport {
        model: model_name.to_string(),
        format: format!("{format:?}"),
        scenarios_total: n,
        parse_success: results.iter().filter(|r| r.parsed).count(),
        tool_name_correct: results.iter().filter(|r| r.correct_tool).count(),
        params_present: results.iter().filter(|r| r.params_present).count(),
        params_type_correct: results.iter().filter(|r| r.params_correct_type).count(),
        avg_latency_ms: if n > 0 {
            total_latency as f64 / n as f64
        } else {
            0.0
        },
        results,
    })
}

/// Resolve a model name to a GGUF file path using the production ModelChoice registry.
fn find_model_path(model_name: &str) -> Result<String, Box<dyn std::error::Error>> {
    use arkavo_router::decision::ModelChoice;

    let choice = ModelChoice::from_name(model_name)
        .ok_or_else(|| format!("Unknown model: '{model_name}'"))?;

    let repo = choice
        .repo_id()
        .ok_or_else(|| format!("No repo for model '{model_name}'"))?;
    let file = choice
        .gguf_filename()
        .ok_or_else(|| format!("No GGUF filename for model '{model_name}'"))?;

    if !arkavo_router::model_discovery::is_model_cached(repo, file) {
        return Err(format!(
            "Model '{model_name}' not cached. Download with: huggingface-cli download {repo} {file}"
        )
        .into());
    }

    // Use the same cache path resolution as production
    let hf_cache = dirs::home_dir()
        .ok_or("No home directory")?
        .join(".cache/huggingface/hub");
    let repo_cache = format!("models--{}", repo.replace('/', "--"));
    let snapshots = hf_cache.join(&repo_cache).join("snapshots");

    for snap in std::fs::read_dir(&snapshots)? {
        let snap = snap?;
        let candidate = snap.path().join(file);
        if candidate.exists() {
            return Ok(candidate.to_string_lossy().to_string());
        }
    }

    Err(format!("GGUF file not found in cache for '{model_name}'").into())
}

/// Discover cached local models using the production ModelChoice registry.
fn discover_cached_models() -> Vec<String> {
    use arkavo_router::decision::ModelChoice;
    use arkavo_router::model_discovery::is_model_cached;

    // All local models in the production registry, ordered smallest → largest
    let candidates = [
        ModelChoice::LocalQwen3,
        ModelChoice::LocalGemma4E2B,
        ModelChoice::LocalMinistral3B,
        // ModelChoice::LocalGemma4E4B, // non-lazy grammar not supported yet (1/8)
        ModelChoice::LocalGlm47Flash,
        ModelChoice::LocalGemma4_26B,
        ModelChoice::LocalMinistral8B,
        ModelChoice::LocalQwen35_9B,
        ModelChoice::LocalQwen35_27B,
    ];

    candidates
        .iter()
        .filter(|m| {
            matches!(
                (m.repo_id(), m.gguf_filename()),
                (Some(repo), Some(file)) if is_model_cached(repo, file)
            )
        })
        .map(|m| m.name().to_string())
        .collect()
}

/// Synthetic fence-format outputs for offline parser testing.
fn synthetic_fence_outputs() -> Vec<String> {
    vec![
        // simple_single_param
        "I'll check the weather.\n```get_weather\nlocation: Tokyo\n```".to_string(),
        // multi_param
        "```search\nquery: rust async\nlimit: 5\n```".to_string(),
        // no_params
        "```get_time\n```".to_string(),
        // enum_param
        "```get_weather\nlocation: Berlin\nunit: fahrenheit\n```".to_string(),
        // file_path
        "```read_file\nfile_path: /etc/hosts\n```".to_string(),
        // command_execution
        "```run_command\ncommand: ls -la /tmp\n```".to_string(),
        // should_not_call
        "Hello! I'm doing great, thanks for asking!".to_string(),
        // multi_param_types
        "```search\nquery: error handling\nlimit: 10\ncase_sensitive: true\n```".to_string(),
    ]
}

/// Tool-loop benchmark: measures the full round-trip of
/// inference1 (tool call) → synthetic result → inference2 (response).
///
/// This reproduces the 62s second-inference bottleneck observed with large models
/// processing tool results. Run with:
///   arkavo tool-bench --tool-loop --model gemma-4-26b-a4b
async fn run_tool_loop_bench(command: &ToolBenchCommand) -> Result<(), Box<dyn std::error::Error>> {
    use arkavo_llm::llamacpp_provider::{LlamaCppProvider, SamplingConfig};
    use arkavo_llm::provider::Provider;

    let model_name = command
        .model
        .as_deref()
        .ok_or("--model required for --tool-loop")?;

    let model_path = find_model_path(model_name)?;
    let registry = arkavo_llm::ModelRegistry::new();
    registry
        .load(model_name, &model_path)
        .map_err(|e| format!("Failed to load model: {e}"))?;

    let config = SamplingConfig::default();
    let provider = LlamaCppProvider::new_with_registry(
        std::sync::Arc::new(registry),
        model_name.to_string(),
        config,
    )?;

    let tools = test_tools();
    let tools_json = arkavo_llm::McpConverter::to_anthropic_format_minimal(
        &tools
            .iter()
            .map(|t| arkavo_mcp_tools::registry::MinimalToolInfo {
                name: t.name.clone(),
                category: Some(t.category.clone()),
                description: Some(t.description.clone()),
                schema: Some(t.schema.clone()),
                aliases: None,
            })
            .collect::<Vec<_>>(),
    );

    // Scenarios: each has a prompt, expected tool call, and a synthetic result
    let loop_scenarios = [
        (
            "weather_loop",
            "What's the weather in Tokyo?",
            "get_weather",
            r#"{"temperature": 22, "condition": "Partly cloudy", "humidity": 65, "wind": "12 km/h NE"}"#,
        ),
        (
            "search_loop",
            "Search for 'rust error handling' with limit 5",
            "search",
            r#"{"results": [{"title": "Error Handling in Rust", "url": "https://doc.rust-lang.org/book/ch09-00-error-handling.html"}, {"title": "anyhow crate", "url": "https://docs.rs/anyhow"}, {"title": "thiserror", "url": "https://docs.rs/thiserror"}, {"title": "Custom Error Types", "url": "https://blog.rust-lang.org/errors"}, {"title": "? operator guide", "url": "https://doc.rust-lang.org/reference/expressions/operator-expr.html"}], "total": 5}"#,
        ),
        (
            "command_loop",
            "Run the command 'ls -la /tmp'",
            "run_command",
            "total 48\ndrwxrwxrwt  12 root  wheel  384 Apr  6 12:00 .\ndrwxr-xr-x  20 root  wheel  640 Mar 15 08:30 ..\n-rw-r--r--   1 user  wheel  1024 Apr  6 11:55 test.txt\n-rw-r--r--   1 user  wheel  2048 Apr  6 10:30 data.json",
        ),
    ];

    println!("Tool-Loop Bench — Model: {model_name}");
    println!("Measures: prompt → tool call → synthetic result → response");
    println!("═══════════════════════════════════════════════════════════════");
    println!(
        "{:<20} {:<10} {:<12} {:<12} {:<12} {:<10}",
        "Scenario", "Tool OK", "Infer1 ms", "Infer2 ms", "Total ms", "Resp len"
    );
    println!("{}", "─".repeat(76));

    let iterations = command.iterations;
    let mut total_infer1 = 0u64;
    let mut total_infer2 = 0u64;

    for (name, prompt, expected_tool, synthetic_result) in &loop_scenarios {
        for _ in 0..iterations {
            // Inference 1: prompt → tool call
            let messages1 = vec![arkavo_llm::Message::user(prompt.to_string())];
            let start1 = Instant::now();
            let resp1 = provider
                .complete_with_tools(messages1, Some(tools_json.clone()), None)
                .await;
            let infer1_ms = start1.elapsed().as_millis() as u64;

            let (tool_ok, _tool_name) = match &resp1 {
                Ok(r) => {
                    let mut calls = arkavo_router::tool_extraction::filter_and_extract_tool_calls(
                        r.tool_calls.clone(),
                    );
                    if calls.is_empty() && !r.content.is_empty() {
                        calls = arkavo_router::tool_extraction::extract_tool_calls_from_text(
                            &r.content,
                        );
                    }
                    let ok = calls.first().is_some_and(|c| c.tool_name == *expected_tool);
                    let name = calls
                        .first()
                        .map(|c| c.tool_name.clone())
                        .unwrap_or_else(|| "-".to_string());
                    (ok, name)
                }
                Err(_) => (false, "ERROR".to_string()),
            };

            // Inference 2: tool call + result → final response
            let mut messages2 = vec![
                arkavo_llm::Message::user(prompt.to_string()),
                arkavo_llm::Message::assistant(format!(
                    "I'll call {expected_tool} to get that information."
                )),
                arkavo_llm::Message::user(format!(
                    "Tool {expected_tool} returned:\n{synthetic_result}"
                )),
            ];

            // Truncate result for very large payloads (mirrors production compaction)
            if messages2.last().map(|m| m.content.len()).unwrap_or(0) > 4000 {
                if let Some(last) = messages2.last_mut() {
                    last.content = last.content[..4000].to_string();
                    last.content.push_str("\n[truncated]");
                }
            }

            let start2 = Instant::now();
            let resp2 = provider.complete_with_tools(messages2, None, None).await;
            let infer2_ms = start2.elapsed().as_millis() as u64;

            let resp_len = match &resp2 {
                Ok(r) => r.content.len(),
                Err(_) => 0,
            };

            total_infer1 += infer1_ms;
            total_infer2 += infer2_ms;

            println!(
                "{:<20} {:<10} {:<12} {:<12} {:<12} {:<10}",
                name,
                if tool_ok { "ok" } else { "FAIL" },
                format!("{infer1_ms}ms"),
                format!("{infer2_ms}ms"),
                format!("{}ms", infer1_ms + infer2_ms),
                format!("{resp_len}ch"),
            );
        }
    }

    let n = loop_scenarios.len() as u64 * iterations as u64;
    println!("{}", "─".repeat(76));
    println!(
        "Average: infer1={:.0}ms  infer2={:.0}ms  total={:.0}ms",
        total_infer1 as f64 / n as f64,
        total_infer2 as f64 / n as f64,
        (total_infer1 + total_infer2) as f64 / n as f64,
    );

    if let Some(ref path) = command.output {
        let report = json!({
            "model": model_name,
            "mode": "tool-loop",
            "scenarios": loop_scenarios.len(),
            "iterations": iterations,
            "avg_infer1_ms": total_infer1 as f64 / n as f64,
            "avg_infer2_ms": total_infer2 as f64 / n as f64,
            "avg_total_ms": (total_infer1 + total_infer2) as f64 / n as f64,
        });
        std::fs::write(path, serde_json::to_string_pretty(&report)?)?;
        println!("Results saved to {path}");
    }

    Ok(())
}

use arkavo_llm::provider::Provider;
