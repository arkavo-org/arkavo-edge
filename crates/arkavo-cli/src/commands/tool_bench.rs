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

    // If a model is specified, run live inference
    if let Some(ref model_name) = command.model {
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

                if let Some(ref path) = command.output {
                    let json = serde_json::to_string_pretty(&report)?;
                    std::fs::write(path, json)?;
                    println!("Results saved to {path}");
                }
            }
            Err(e) => {
                eprintln!("Live bench failed: {e}");
            }
        }
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
                        let calls: Vec<_> = resp
                            .tool_calls
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
                        (parsed, correct_tool, pp, ptc, resp.content, ptool, pargs)
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

fn find_model_path(model_name: &str) -> Result<String, Box<dyn std::error::Error>> {
    // Check HuggingFace cache
    let hf_cache = dirs::home_dir()
        .ok_or("No home directory")?
        .join(".cache/huggingface/hub");

    if !hf_cache.exists() {
        return Err(format!("HuggingFace cache not found at {}", hf_cache.display()).into());
    }

    let name_lower = model_name.to_lowercase();

    // Search for matching GGUF files
    for entry in std::fs::read_dir(&hf_cache)? {
        let entry = entry?;
        let dir_name = entry.file_name().to_string_lossy().to_lowercase();
        if !dir_name.contains(&name_lower) && !name_lower.split('-').all(|p| dir_name.contains(p)) {
            continue;
        }

        let snapshots = entry.path().join("snapshots");
        if !snapshots.exists() {
            continue;
        }

        // Find the latest snapshot with a GGUF file
        for snap in std::fs::read_dir(&snapshots)? {
            let snap = snap?;
            for file in std::fs::read_dir(snap.path())? {
                let file = file?;
                let fname = file.file_name().to_string_lossy().to_lowercase();
                if fname.ends_with(".gguf") {
                    return Ok(file.path().to_string_lossy().to_string());
                }
            }
        }
    }

    Err(format!("No GGUF model found for '{model_name}' in HuggingFace cache").into())
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

use arkavo_llm::provider::Provider;
