use clap::Args;
use serde_json::{Value, json};
use std::time::Instant;

use arkavo_llm::mcp_converter::LocalToolFormat;
use arkavo_llm::tool_parser::ToolParser;
use arkavo_mcp_tools::registry::ToolInfo;

#[derive(Args)]
pub struct ToolBenchCommand {
    /// Model name to test (e.g., "qwen3-0.6b", "ministral-3b", "glm-5.2")
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

    /// Include cloud models in `--all`. Cloud calls cost money and require the
    /// relevant API key (e.g. GLM_API_KEY). On by default; pass
    /// `--no-include-cloud` to restrict to locally-cached GGUF models.
    #[arg(long, default_value = "true", action = clap::ArgAction::Set)]
    pub include_cloud: bool,
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
    /// Provider-reported prompt tokens (cloud arms from the response `usage`
    /// block; local arms stay 0 — llama.cpp token counts aren't surfaced here).
    prompt_tokens: u32,
    /// Provider-reported completion tokens.
    completion_tokens: u32,
    /// Estimated USD cost for this single call at the model's published rate.
    /// Local models are free (0.0).
    estimated_cost_usd: f64,
    /// True when the scenario was *not attempted* due to a transient provider
    /// error (429 rate-limit / account-balance, network, timeout) rather than a
    /// genuine model capability failure. Skipped scenarios are excluded from the
    /// accuracy counts so a single rate-limit can't read as a capability gap —
    /// the central credibility requirement for a paid-model benchmark.
    skipped: bool,
}

#[derive(serde::Serialize)]
struct ModelReport {
    model: String,
    format: String,
    /// `local` (GGUF via llama.cpp) or `cloud` (remote API via the Router's
    /// provider path). Lets a reader split the comparison table at a glance.
    deployment: &'static str,
    scenarios_total: usize,
    /// Scenarios the model actually got to attempt (total minus skipped).
    /// Accuracy counts (`parse_success`, `tool_name_correct`, …) are computed
    /// over the full `results` set, so when `scenarios_skipped > 0` the honest
    /// accuracy is `count / scenarios_attempted`, not `count / total`.
    scenarios_attempted: usize,
    scenarios_skipped: usize,
    parse_success: usize,
    tool_name_correct: usize,
    params_present: usize,
    params_type_correct: usize,
    avg_latency_ms: f64,
    /// Sum of per-scenario estimated_cost_usd. The real spend signal for cloud
    /// arms; 0.0 for local. Populated from provider `usage`, not category
    /// priors, so it tracks actual consumption.
    total_cost_usd: f64,
    results: Vec<ScenarioResult>,
}

impl ModelReport {
    /// Build a report from a completed result set, computing all aggregate
    /// counts consistently. Accuracy counts are taken over the **attempted**
    /// scenarios (total minus skipped) so a transient 429 can't depress the
    /// accuracy figure — `parse_success / scenarios_attempted` is the honest
    /// success rate; `scenarios_total` is preserved for transparency.
    fn from_results(
        model: String,
        format: String,
        deployment: &'static str,
        results: Vec<ScenarioResult>,
    ) -> Self {
        let attempted = results.iter().filter(|r| !r.skipped).count();
        Self {
            model,
            format,
            deployment,
            scenarios_total: results.len(),
            scenarios_attempted: attempted,
            scenarios_skipped: results.len() - attempted,
            parse_success: results.iter().filter(|r| r.parsed).count(),
            tool_name_correct: results.iter().filter(|r| r.correct_tool).count(),
            params_present: results.iter().filter(|r| r.params_present).count(),
            params_type_correct: results.iter().filter(|r| r.params_correct_type).count(),
            avg_latency_ms: avg_latency(&results),
            total_cost_usd: results.iter().map(|r| r.estimated_cost_usd).sum(),
            results,
        }
    }
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

    // Determine which models to bench. `--model` honors either kind; `--all`
    // expands to cached locals plus (optionally) available cloud arms.
    let models: Vec<String> = if command.all {
        let mut found = discover_cached_models();
        if command.include_cloud {
            found.extend(discover_cloud_models());
        }
        found
    } else if let Some(ref model_name) = command.model {
        vec![model_name.clone()]
    } else {
        vec![]
    };

    let mut all_reports = Vec::new();
    for model_name in &models {
        let deployment = model_deployment(model_name);
        println!("\n═══════════════════════════════════════════════════════════════");
        println!("Live inference with model: {model_name} ({deployment})");
        println!("═══════════════════════════════════════════════════════════════");

        let report_result: Result<Option<ModelReport>, Box<dyn std::error::Error>> =
            match deployment {
                "cloud" => {
                    run_cloud_bench(model_name, &tools, &scenarios, command.iterations).await
                }
                _ => run_live_bench(model_name, &tools, &scenarios, format, command.iterations)
                    .await
                    .map(Some),
            };

        match report_result {
            Ok(Some(report)) => {
                let cost_str = if report.total_cost_usd > 0.0 {
                    format!("  Cost: ${:.5}", report.total_cost_usd)
                } else {
                    String::new()
                };
                // Denominator is attempted (not total) so a skipped scenario
                // from a transient 429 can't read as a miss. When scenarios
                // were skipped, surface that explicitly — a reader comparing
                // 7/7 vs 8/8 needs to know the 7 was over fewer attempts.
                let denom = report.scenarios_attempted;
                let skip_note = if report.scenarios_skipped > 0 {
                    format!(
                        "  ({} skipped: transient provider error)",
                        report.scenarios_skipped
                    )
                } else {
                    String::new()
                };
                println!(
                    "\nResults: Parse {}/{}  Tool {}/{}  Params {}/{}  Avg latency: {:.0}ms{cost_str}{skip_note}",
                    report.parse_success,
                    denom,
                    report.tool_name_correct,
                    denom,
                    report.params_present,
                    denom,
                    report.avg_latency_ms,
                );
                all_reports.push(report);
            }
            Ok(None) => {
                // Cloud arm skipped (no key/feature) — already logged above.
            }
            Err(e) => {
                eprintln!("Live bench failed for {model_name}: {e}");
            }
        }
    }

    if all_reports.len() > 1 {
        println!("\n═══════════════════════════════════════════════════════════════");
        println!("Summary");
        println!("═══════════════════════════════════════════════════════════════");
        println!(
            "{:<22} {:<7} {:<14} {:<14} {:<14} {:<10}",
            "Model", "Kind", "Parse", "Tool", "Params", "Avg ms"
        );
        println!("{}", "─".repeat(85));
        for r in &all_reports {
            // "count/attempted" — the honest success rate. Skipped scenarios
            // are excluded from the denominator (annotated if any).
            let denom = r.scenarios_attempted;
            let parse = format!("{}/{}", r.parse_success, denom);
            let tool = format!("{}/{}", r.tool_name_correct, denom);
            let params = format!("{}/{}", r.params_present, denom);
            println!(
                "{:<22} {:<7} {:<14} {:<14} {:<14} {:.0}",
                r.model, r.deployment, parse, tool, params, r.avg_latency_ms,
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

/// "local" for GGUF-backed arms (benched via llama.cpp), "cloud" for remote
/// API arms (benched via the Router provider path). Drives the branch in run().
fn model_deployment(model_name: &str) -> &'static str {
    use arkavo_router::decision::ModelChoice;
    match ModelChoice::from_name(model_name) {
        Some(m) if m.is_cloud() => "cloud",
        _ => "local",
    }
}

/// Cloud arms that are actually runnable right now (feature + key present).
/// Only models we can score end-to-end are returned, so `--all --include-cloud`
/// never launches a paid call it can't validate.
fn discover_cloud_models() -> Vec<String> {
    use arkavo_router::decision::ModelChoice;
    // Candidate cloud arms, in cost-ascending order so the summary lists the
    // cheapest comparison point (GLM-5.2) first.
    let candidates = [ModelChoice::Glm52];
    candidates
        .iter()
        .filter(|m| cloud_arm_available(m))
        .map(|m| m.name().to_string())
        .collect()
}

async fn run_live_bench(
    model_name: &str,
    tools: &[ToolInfo],
    scenarios: &[Scenario],
    format: LocalToolFormat,
    iterations: usize,
) -> Result<ModelReport, Box<dyn std::error::Error>> {
    use arkavo_llm::llamacpp_provider::{LlamaCppProvider, SamplingConfig};
    use arkavo_router::decision::ModelChoice;

    let config = if let Some((temp, top_p, thinking)) =
        ModelChoice::from_name(model_name).and_then(|m| m.optimal_sampling())
    {
        SamplingConfig {
            temperature: temp,
            top_p,
            thinking_mode: Some(thinking),
            tool_format: format,
            ..SamplingConfig::default()
        }
    } else {
        SamplingConfig {
            tool_format: format,
            ..SamplingConfig::default()
        }
    };

    let registry = arkavo_llm::ModelRegistry::new();
    let model_path = find_model_path(model_name)?;
    registry
        .load(model_name, &model_path)
        .map_err(|e| format!("Failed to load model '{model_name}': {e}"))?;

    // Pre-warm context pool (same as production agent loop)
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    if let Ok(ctx) = registry.acquire_fresh_context(model_name) {
        let _ = registry.release_context(model_name, ctx, true);
        println!("Context pool pre-warmed for {model_name}");
    }

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

    let results = run_scenario_suite(
        &provider,
        &tools_json,
        &registered,
        scenarios,
        tools,
        iterations,
        model_name,
    )
    .await?;

    Ok(ModelReport::from_results(
        model_name.to_string(),
        format!("{format:?}"),
        "local",
        results,
    ))
}

/// Benchmark a **cloud** model arm (GLM-5.2, and any future OpenAI-compatible
/// cloud arm) through the Router's exact provider path — `get_provider(model)`
/// → `complete_with_tools`. This is the same instantiation the production
/// agent loop uses, so the numbers reflect real routing, not a bespoke client.
///
/// Returns `Ok(None)` (rather than erroring) when the arm isn't runnable in
/// this environment — missing API key or disabled feature — so `--all` skips
/// it cleanly like an uncached local model, instead of failing the whole run.
async fn run_cloud_bench(
    model_name: &str,
    tools: &[ToolInfo],
    scenarios: &[Scenario],
    iterations: usize,
) -> Result<Option<ModelReport>, Box<dyn std::error::Error>> {
    use arkavo_router::decision::ModelChoice;

    let model = ModelChoice::from_name(model_name)
        .ok_or_else(|| format!("Unknown model: '{model_name}'"))?;

    if !model.is_cloud() {
        return Err(format!("run_cloud_bench called with non-cloud model '{model_name}'").into());
    }

    // Gate exactly like the router does: feature flag + API key. Skipping here
    // (not erroring) lets `--all` fan out across local + cloud without a missing
    // key aborting the whole benchmark.
    if !cloud_arm_available(&model) {
        println!(
            "  Skipping cloud model {model_name}: feature/key not available (set e.g. GLM_API_KEY to enable)"
        );
        return Ok(None);
    }

    println!("Initializing Router for cloud model {model_name}...");
    let init_start = Instant::now();
    let router = arkavo_router::Router::new_offline()
        .await
        .map_err(|e| format!("Failed to initialize router: {e}"))?;
    let router = std::sync::Arc::new(router);
    println!("Router ready in {:.1}s", init_start.elapsed().as_secs_f64());

    let provider = router
        .get_provider(&model)
        .await
        .map_err(|e| format!("Failed to instantiate cloud provider for {model_name}: {e}"))?;

    // Cloud arms speak the provider's native tool schema (OpenAI function-calling
    // for GLM-5.2), not the local fence format — to_anthropic_format_minimal is
    // the generic tool list the OpenAIProvider converts per-provider on the wire.
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

    let results = run_scenario_suite(
        provider.as_ref(),
        &tools_json,
        &registered,
        scenarios,
        tools,
        iterations,
        model_name,
    )
    .await?;

    Ok(Some(ModelReport::from_results(
        model_name.to_string(),
        // Cloud arms ignore the local fence/xml/json `format` — the provider
        // speaks its native function-calling schema. Record "native" so the
        // report doesn't misleadingly claim a fence format was used.
        "native".to_string(),
        "cloud",
        results,
    )))
}

/// Run the 8-scenario suite against any provider that implements
/// `complete_with_tools`, applying the **same** production post-processing and
/// the **same** param validation for local and cloud. Sharing this is what
/// makes the two tables directly comparable — a cloud cell and a local cell in
/// the same column are scored identically.
///
/// Also surfaces real token usage + cost per call from the provider response
/// (`InferenceTiming` for cloud arms; zeros for local).
async fn run_scenario_suite(
    provider: &dyn arkavo_llm::Provider,
    tools_json: &Value,
    registered: &std::collections::HashSet<&str>,
    scenarios: &[Scenario],
    _tools: &[ToolInfo],
    iterations: usize,
    model_name: &str,
) -> Result<Vec<ScenarioResult>, Box<dyn std::error::Error>> {
    let mut results = Vec::new();
    // Cost rate for cloud arms, looked up once. Local arms return None and each
    // call is priced 0.0. We estimate from real per-call token counts (when the
    // provider surfaces them) so the total tracks actual consumption, not the
    // category-prior estimate the router uses for planning.
    let cost_estimator = CostEstimator::for_model_name(model_name);

    for scenario in scenarios {
        for _ in 0..iterations {
            let messages = vec![arkavo_llm::Message::user(scenario.prompt.to_string())];

            let start = Instant::now();
            let response = provider
                .complete_with_tools(messages, Some(tools_json.clone()), None)
                .await;
            let latency = start.elapsed().as_millis() as u64;

            let (
                parsed,
                correct_tool,
                params_present,
                params_type_correct,
                raw,
                ptool,
                pargs,
                prompt_tokens,
                completion_tokens,
                skipped,
            ) = match response {
                Ok(resp) => {
                    // Production post-processing, in order:
                    // 1. Filter language fences (e.g. ```python\ntool(...)```).
                    let mut calls = arkavo_router::tool_extraction::filter_and_extract_tool_calls(
                        resp.tool_calls,
                    );
                    // 2. If nothing parsed, try text-extraction fallbacks
                    //    (curly-brace, Python-style, XML, JSON).
                    if calls.is_empty() && !resp.content.is_empty() {
                        calls = arkavo_router::tool_extraction::extract_tool_calls_from_text(
                            &resp.content,
                        );
                    }
                    // 3. Keep only registered tool names.
                    let calls: Vec<_> = calls
                        .into_iter()
                        .filter(|c| registered.contains(c.tool_name.as_str()))
                        .collect();

                    let parsed = !calls.is_empty() || scenario.expected_tool.is_none();
                    let correct_tool = match scenario.expected_tool {
                        Some(exp) => calls.first().is_some_and(|c| c.tool_name == exp),
                        None => calls.is_empty(),
                    };

                    let (pp, ptc) = evaluate_params(&calls, scenario);

                    let ptool = calls.first().map(|c| c.tool_name.clone());
                    let pargs = calls.first().map(|c| c.arguments.clone());
                    let clean_content = arkavo_router::response::strip_think_blocks(&resp.content);
                    let (pt, ct) = resp
                        .inference_timing
                        .map(|t| (t.n_prompt_eval, t.n_eval))
                        .unwrap_or((0, 0));
                    (
                        parsed,
                        correct_tool,
                        pp,
                        ptc,
                        clean_content,
                        ptool,
                        pargs,
                        pt,
                        ct,
                        // A successful HTTP round-trip is a real attempt, even
                        // if the model produced no tool call — never skipped.
                        false,
                    )
                }
                Err(e) => {
                    let msg = format!("{e}");
                    // Distinguish "the model couldn't do it" from "the provider
                    // never let the model try". A 429 / account-balance / network
                    // / timeout error is transient infrastructure, not capability
                    // — flagging it skipped keeps a rate-limit from reading as a
                    // tool-calling failure in the accuracy counts.
                    let skipped = is_transient_provider_error(&msg);
                    let label = if skipped { "SKIPPED" } else { "ERROR" };
                    (
                        false,
                        false,
                        false,
                        false,
                        format!("{label}: {msg}"),
                        None,
                        None,
                        0,
                        0,
                        skipped,
                    )
                }
            };

            let estimated_cost_usd = cost_estimator.estimate(prompt_tokens, completion_tokens);

            let status = if skipped {
                "⊘"
            } else if correct_tool && params_present {
                "✓"
            } else {
                "✗"
            };
            let cost_str = if estimated_cost_usd > 0.0 {
                format!("${estimated_cost_usd:.5}")
            } else {
                "-".to_string()
            };
            println!(
                "  {status} {:<25} tool={:<15} latency={latency}ms tokens={prompt_tokens}+{completion_tokens} cost={cost_str}",
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
                prompt_tokens,
                completion_tokens,
                estimated_cost_usd,
                skipped,
            });
        }
    }

    Ok(results)
}

/// Evaluate the parsed tool calls against a scenario's expected params.
/// Shared by local and cloud so a `params_present` cell means the same thing
/// in both columns of the comparison table.
fn evaluate_params(
    calls: &[arkavo_llm::tool_parser::ParsedToolCall],
    scenario: &Scenario,
) -> (bool, bool) {
    let Some(call) = calls.first() else {
        return (
            scenario.expected_tool.is_none(),
            scenario.expected_tool.is_none(),
        );
    };
    let pp = scenario
        .expected_params
        .iter()
        .all(|(n, _)| call.arguments.get(*n).is_some());
    let ptc = scenario
        .expected_params
        .iter()
        .all(|(n, check)| match check {
            ParamCheck::Present => call.arguments.get(*n).is_some(),
            ParamCheck::IsType(t) => call.arguments.get(*n).is_some_and(|v| match *t {
                "number" => v.is_number(),
                "boolean" => v.is_boolean(),
                "string" => v.is_string(),
                _ => true,
            }),
        });
    (pp, ptc)
}

fn avg_latency(results: &[ScenarioResult]) -> f64 {
    if results.is_empty() {
        return 0.0;
    }
    let total: u64 = results.iter().map(|r| r.latency_ms).sum();
    total as f64 / results.len() as f64
}

/// Whether a cloud arm can actually be instantiated in this environment —
/// feature enabled AND the matching API key present. Mirrors the router's
/// `is_model_available` gating so the bench never attempts a call that would
/// dead-end on a missing key/feature.
fn cloud_arm_available(model: &arkavo_router::decision::ModelChoice) -> bool {
    use arkavo_router::decision::ModelChoice;
    match model {
        // cfg! evaluates at compile time of the *cli* crate, which forwards the
        // `glm` feature — so this correctly reflects whether the GLM path was
        // compiled in, matching the router's `#[cfg(feature = "glm")]` gate.
        ModelChoice::Glm52 => cfg!(feature = "glm") && std::env::var("GLM_API_KEY").is_ok(),
        // Other cloud arms are benched via the same path once their provider
        // instantiation is wired into run_cloud_bench; until then report them
        // as unavailable so --all doesn't attempt paid calls it can't score.
        _ => false,
    }
}

/// Per-call USD estimation from real token counts. Cloud arms carry a
/// published per-MTok rate; local arms return 0.0 (free). Keeping this inline
/// (rather than calling RoutingDecision::estimate_cost, which uses category
/// priors) means the reported cost tracks *actual* consumption per scenario.
struct CostEstimator {
    input_per_mtok: f64,
    output_per_mtok: f64,
}

impl CostEstimator {
    fn for_model_name(name: &str) -> Self {
        use arkavo_router::decision::ModelChoice;
        // Match RoutingDecision::estimate_cost's published rates so the
        // per-call figure reconciles with the router's planning estimate.
        let (i, o) = match ModelChoice::from_name(name) {
            Some(ModelChoice::Glm52) => (1.40, 4.40),
            // Local and unmapped arms: free / unknown → 0 so the column reads
            // honestly rather than inventing a rate.
            _ => (0.0, 0.0),
        };
        Self {
            input_per_mtok: i,
            output_per_mtok: o,
        }
    }

    fn estimate(&self, prompt_tokens: u32, completion_tokens: u32) -> f64 {
        // mul_add avoids a separate rounding from the multiply-then-add; the
        // published per-MTok math must be bit-stable so the reported cost
        // reconciles exactly with RoutingDecision::estimate_cost.
        (completion_tokens as f64 / 1_000_000.0).mul_add(
            self.output_per_mtok,
            (prompt_tokens as f64 / 1_000_000.0) * self.input_per_mtok,
        )
    }
}

/// Classify a provider error string as transient infrastructure (rate-limit,
/// account balance, network, timeout, server error) rather than a genuine model
/// capability failure. Matched against the raw error text because the cloud
/// provider path surfaces errors as `anyhow` strings (not typed
/// `ProviderError`); the patterns cover the strings the OpenAI-compatible
/// adapter emits for each retryable class.
///
/// This is what keeps a single 429 from flunking a model in the accuracy
/// counts — the scenario is marked `skipped` and excluded, so the reported
/// accuracy reflects only calls the model actually got to answer.
fn is_transient_provider_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    // 429 rate-limit, including the "insufficient balance / recharge" variant
    // Z.ai returns under the same status — the account can't serve the call
    // right now, but that's a billing/throughput state, not model quality.
    lower.contains("429")
        || lower.contains("too many requests")
        || lower.contains("rate limit")
        || lower.contains("insufficient balance")
        || lower.contains("recharge")
        || lower.contains("no resource package")
        // 5xx server-side / availability
        || lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
        || lower.contains("bad gateway")
        || lower.contains("service unavailable")
        || lower.contains("gateway timeout")
        || lower.contains("internal server error")
        // Connectivity / timeout — the request never reached a model
        || lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("connection")
        || lower.contains("network")
        || lower.contains("dns")
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
        ModelChoice::LocalGemma4_12B,
        ModelChoice::LocalQwen35_27B,
        ModelChoice::LocalQwen36A3B,
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
    let model_name = command
        .model
        .as_deref()
        .ok_or("--model required for --tool-loop")?;

    let model_choice = arkavo_router::decision::ModelChoice::from_name(model_name)
        .ok_or_else(|| format!("Unknown model: '{model_name}'"))?;

    // Use the production Router (same as `arkavo chat`) — handles model loading,
    // context pool warmup, Metal shader compilation, and inference semaphore.
    println!("Initializing Router (model loading + context warmup)...");
    let init_start = Instant::now();
    let router = arkavo_router::Router::new_offline()
        .await
        .map_err(|e| format!("Failed to initialize router: {e}"))?;
    let router = std::sync::Arc::new(router);
    println!("Router ready in {:.1}s", init_start.elapsed().as_secs_f64());

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
        "{:<20} {:<10} {:<12} {:<12} {:<12} {:<10} {:<10}",
        "Scenario", "Tool OK", "Infer1 ms", "Infer2 ms", "Total ms", "Resp len", "Spec acc %"
    );
    println!("{}", "─".repeat(86));

    let iterations = command.iterations;
    let mut total_infer1 = 0u64;
    let mut total_infer2 = 0u64;

    for (name, prompt, expected_tool, synthetic_result) in &loop_scenarios {
        for _ in 0..iterations {
            // Inference 1: prompt → tool call (via Router, same as agent loop)
            let messages1 = vec![arkavo_llm::Message::user(prompt.to_string())];
            let start1 = Instant::now();
            let resp1 = router
                .route_with_tools_override(prompt, messages1, None, &model_choice)
                .await;
            let infer1_ms = start1.elapsed().as_millis() as u64;

            let tool_ok = match &resp1 {
                Ok(r) => r
                    .tool_calls
                    .first()
                    .is_some_and(|c| c.tool_name == *expected_tool),
                Err(_) => false,
            };

            // Inference 2: tool call + result → final response
            let messages2 = vec![
                arkavo_llm::Message::user(prompt.to_string()),
                arkavo_llm::Message::assistant(format!(
                    "I'll call {expected_tool} to get that information."
                )),
                arkavo_llm::Message::user(format!(
                    "Tool {expected_tool} returned:\n{synthetic_result}"
                )),
            ];

            let start2 = Instant::now();
            let resp2 = router
                .route_with_tools_override(
                    &format!("Process the {expected_tool} result"),
                    messages2,
                    None,
                    &model_choice,
                )
                .await;
            let infer2_ms = start2.elapsed().as_millis() as u64;

            let (resp_len, spec_acc) = match &resp2 {
                Ok(r) => {
                    let len = r.content.len();
                    let spec_str = match r.inference_timing.as_ref() {
                        Some(timing) => match (
                            timing.spec_bypassed.as_deref(),
                            timing.n_draft,
                            timing.n_accepted,
                        ) {
                            (Some(reason), _, _) => {
                                format!("byp:{}", reason.chars().take(5).collect::<String>())
                            }
                            (None, Some(d), Some(a)) if d > 0 => format!("{}%", (a * 100) / d),
                            (None, Some(_), Some(_)) => "0%".to_string(),
                            (None, _, _) => "-".to_string(),
                        },
                        None => "-".to_string(),
                    };
                    (len, spec_str)
                }
                Err(_) => (0, "-".to_string()),
            };

            total_infer1 += infer1_ms;
            total_infer2 += infer2_ms;

            println!(
                "{:<20} {:<10} {:<12} {:<12} {:<12} {:<10} {:<10}",
                name,
                if tool_ok { "ok" } else { "FAIL" },
                format!("{infer1_ms}ms"),
                format!("{infer2_ms}ms"),
                format!("{}ms", infer1_ms + infer2_ms),
                format!("{resp_len}ch"),
                spec_acc,
            );
        }
    }

    let n = loop_scenarios.len() as u64 * iterations as u64;
    println!("{}", "─".repeat(86));
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

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_llm::tool_parser::ParsedToolCall;
    use arkavo_router::decision::ModelChoice;
    use serde_json::json;

    // GLM-5.2 is cloud; a local GGUF arm is not. This guards the branch in
    // run() that sends each arm to the right provider path — a regression here
    // would route a cloud call through the GGUF loader (or vice versa).
    #[test]
    fn model_deployment_classifies_cloud_and_local() {
        assert_eq!(model_deployment("glm-5.2"), "cloud");
        assert_eq!(model_deployment("GLM-5.2"), "cloud");
        assert_eq!(model_deployment("ministral-3b"), "local");
        assert_eq!(model_deployment("qwen3.5-0.8b"), "local");
        // Unknown names fall through to local so a typo doesn't crash the bench;
        // run_live_bench will surface a clear "unknown model" error instead.
        assert_eq!(model_deployment("not-a-real-model"), "local");
    }

    // The shared evaluator must score the SAME regardless of which provider
    // produced the call. A cloud ParsedToolCall and a synthetic one carrying
    // identical name/arguments must both pass — this is the comparability
    // guarantee between the local and cloud columns of the benchmark table.
    #[test]
    fn evaluate_params_is_provider_agnostic() {
        let scenario = Scenario {
            name: "multi_param_types",
            prompt: "",
            expected_tool: Some("search"),
            expected_params: vec![
                ("query", ParamCheck::Present),
                ("limit", ParamCheck::IsType("number")),
                ("case_sensitive", ParamCheck::IsType("boolean")),
            ],
        };

        // Shaped exactly like what OpenAIProvider::parse_tool_calls emits after
        // the arguments JSON string is deserialized.
        let call = ParsedToolCall {
            tool_name: "search".to_string(),
            arguments: json!({"query": "error handling", "limit": 10, "case_sensitive": true}),
            call_id: Some("call_abc".to_string()),
        };
        let (present, types_ok) = evaluate_params(&[call], &scenario);
        assert!(present, "all expected params present");
        assert!(types_ok, "number + boolean types correct");

        // Missing a param fails present; wrong type fails types_ok but not present.
        let partial = ParsedToolCall {
            tool_name: "search".to_string(),
            arguments: json!({"query": "x", "case_sensitive": "true"}),
            call_id: None,
        };
        let (present, types_ok) = evaluate_params(&[partial], &scenario);
        assert!(!present, "missing `limit` must fail params_present");
        assert!(!types_ok, "string where boolean expected must fail types");
    }

    // A should_not_call scenario: zero calls is the *correct* outcome. The
    // evaluator must return (true, true) here, not penalize the empty result —
    // otherwise every clean text reply scores as a failure.
    #[test]
    fn evaluate_params_no_call_scenario_passes_on_empty() {
        let scenario = Scenario {
            name: "should_not_call",
            prompt: "",
            expected_tool: None,
            expected_params: vec![],
        };
        let (present, types_ok) = evaluate_params(&[], &scenario);
        assert!(present);
        assert!(types_ok);
    }

    // Cloud discovery must ONLY surface arms that are actually runnable
    // (feature + key present), so `--all --include-cloud` never launches a paid
    // call it can't score. We assert the contract rather than a specific count:
    // every discovered name must (a) resolve to a cloud ModelChoice and (b)
    // itself report as available. This holds regardless of whether a dev shell
    // happens to have GLM_API_KEY set, so the test is stable under `cargo
    // nextest` parallelism and on contributor machines.
    #[test]
    fn discover_cloud_models_only_returns_runnable_arms() {
        let found = discover_cloud_models();
        for name in &found {
            let model = ModelChoice::from_name(name)
                .unwrap_or_else(|| panic!("discovered cloud name '{name}' must resolve"));
            assert!(
                model.is_cloud(),
                "discovered name '{name}' must be a cloud arm"
            );
            assert!(
                cloud_arm_available(&model),
                "discovered name '{name}' must report as available"
            );
        }
        // The candidate set is exactly GLM-5.2 today; discovery can't invent
        // arms the bench can't score.
        let all_candidates_resolve_to_glm = found
            .iter()
            .all(|n| ModelChoice::from_name(n) == Some(ModelChoice::Glm52));
        assert!(
            all_candidates_resolve_to_glm,
            "only GLM-5.2 is a candidate cloud arm, got {found:?}"
        );
    }

    // cloud_arm_available must reject arms the bench can't score. Non-GLM cloud
    // arms (Claude/Gemini/DeepSeek/Kimi) aren't wired into run_cloud_bench, so
    // they must report unavailable even when their key is set — otherwise
    // --include-cloud would dispatch a paid call that returns an unhandled
    // provider type.
    #[test]
    fn cloud_arm_available_rejects_unwired_cloud_arms() {
        assert!(
            !cloud_arm_available(&ModelChoice::ClaudeSonnet),
            "Claude isn't wired into the bench path yet"
        );
        assert!(
            !cloud_arm_available(&ModelChoice::GeminiFlash),
            "Gemini isn't wired into the bench path yet"
        );
        assert!(
            !cloud_arm_available(&ModelChoice::DeepSeekV32),
            "DeepSeek isn't wired into the bench path yet"
        );
        // Local arms are never "cloud available" by definition.
        assert!(!cloud_arm_available(&ModelChoice::LocalMinistral3B));
    }

    // The reported cost must reconcile to the published GLM-5.2 rate
    // ($1.40/MTok in, $4.40/MTok out), matching RoutingDecision::estimate_cost's
    // per-MTok figure. A drift here would make the benchmark under/over-report
    // real spend — the central credibility claim for a paid model.
    #[test]
    fn cost_estimator_matches_published_glm52_rate() {
        let est = CostEstimator::for_model_name("glm-5.2");
        // 1M input + 1M output = $1.40 + $4.40 exactly.
        let cost = est.estimate(1_000_000, 1_000_000);
        assert!(
            (cost - 5.80).abs() < 1e-9,
            "1M+1M tokens at GLM-5.2 rate should be $5.80, got {cost}"
        );
        // A realistic small call: 800 in + 3000 out. Mirror the estimator's
        // mul_add form so this is a bit-exact check, not an approximation.
        let small = est.estimate(800, 3000);
        let expected: f64 = (3000.0_f64 / 1_000_000.0).mul_add(4.40, 800.0 / 1_000_000.0 * 1.40);
        assert!(
            (small - expected).abs() < 1e-12,
            "small call cost {small} must equal {expected}"
        );
    }

    // Local arms must report zero cost — never an invented rate. This is what
    // keeps the cost column honest: a 0.0 there means "free local inference",
    // not "unknown".
    #[test]
    fn cost_estimator_local_arms_are_free() {
        for local in ["ministral-3b", "qwen3.5-0.8b", "glm-4.7-flash"] {
            let est = CostEstimator::for_model_name(local);
            assert!(
                est.estimate(1_000_000, 1_000_000) == 0.0,
                "local model {local} must be free, not priced"
            );
        }
    }

    // The report schema must carry the cost/token/deployment/skip fields so the
    // JSON output is self-describing, and `from_results` must populate the
    // attempted/total accounting correctly. This is the regression guard for the
    // central credibility property: a skipped scenario never counts against
    // accuracy.
    #[test]
    fn model_report_from_results_accounts_for_skipped() {
        // A clean local run: 8 attempted, 0 skipped, 8/8 across the board.
        let clean = vec![pass_result("simple_single_param", 690, 0.0)];
        let local_report = ModelReport::from_results(
            "ministral-3b".to_string(),
            "Fence".to_string(),
            "local",
            clean,
        );
        assert_eq!(local_report.deployment, "local");
        assert_eq!(local_report.scenarios_total, 1);
        assert_eq!(local_report.scenarios_attempted, 1);
        assert_eq!(local_report.scenarios_skipped, 0);
        assert_eq!(local_report.parse_success, 1);
        assert!((local_report.avg_latency_ms - 690.0).abs() < 1e-9);
        let json_str = serde_json::to_string(&local_report).unwrap();
        assert!(json_str.contains(r#""deployment":"local""#));
        assert!(json_str.contains(r#""scenarios_attempted":1"#));
        assert!(json_str.contains(r#""skipped":false"#));

        // A cloud run where one scenario hit a 429: total=2, attempted=1,
        // skipped=1. Accuracy counts must reflect the one attempted call, and
        // the report must surface the skip so a reader doesn't read "1/2" as
        // "50% accurate" when really it's 1/1 attempted.
        let mixed = vec![
            pass_result("simple_single_param", 1200, 0.00078),
            skipped_result("multi_param_types", 9055),
        ];
        let cloud_report =
            ModelReport::from_results("glm-5.2".to_string(), "native".to_string(), "cloud", mixed);
        assert_eq!(cloud_report.deployment, "cloud");
        assert_eq!(cloud_report.scenarios_total, 2);
        assert_eq!(
            cloud_report.scenarios_attempted, 1,
            "the 429'd one is excluded"
        );
        assert_eq!(cloud_report.scenarios_skipped, 1);
        assert_eq!(cloud_report.parse_success, 1);
        assert_eq!(cloud_report.tool_name_correct, 1);
        // Cost excludes the skipped call (it carried 0 tokens/cost), so total
        // reflects only real spend.
        assert!(
            (cloud_report.total_cost_usd - 0.00078).abs() < 1e-6,
            "total cost {} should exclude the skipped (0-token) call",
            cloud_report.total_cost_usd
        );
        let cloud_json = serde_json::to_string(&cloud_report).unwrap();
        assert!(cloud_json.contains(r#""deployment":"cloud""#));
        assert!(cloud_json.contains(r#""format":"native""#));
        assert!(cloud_json.contains(r#""scenarios_skipped":1"#));
        assert!(cloud_json.contains(r#""skipped":true"#));
    }

    // Regression for the exact failure observed in the real GLM-5.2 run: the
    // 8th scenario returned "429 Too Many Requests: Insufficient balance ...
    // recharge". That string MUST classify as transient (skipped), not as a
    // capability failure — otherwise the report would have read "7/8" and
    // mischaracterized a billing state as a model-quality gap.
    #[test]
    fn transient_classifier_flags_real_glm52_429_balance_error() {
        let real = "Provider error: OpenAI API error 429 Too Many Requests: \
                   {\"error\":{\"code\":\"1113\",\"message\":\"Insufficient balance \
                   or no resource package. Please recharge.\"}}";
        assert!(
            is_transient_provider_error(real),
            "the observed GLM-5.2 429/balance error must classify as transient"
        );
    }

    // The classifier must cover each retryable class (rate-limit, balance,
    // 5xx, network, timeout) and, just as importantly, must NOT flag genuine
    // capability/request errors (400 bad request, 401 auth, 404 model) as
    // transient — those are real failures a benchmark should count.
    #[test]
    fn transient_classifier_distinguishes_infra_from_capability() {
        let transient = [
            "429 Too Many Requests",
            "rate limit exceeded",
            "Insufficient balance",
            "Please recharge",
            "503 Service Unavailable",
            "502 Bad Gateway",
            "request timed out",
            "connection reset by peer",
            "dns resolution failed",
        ];
        for msg in transient {
            assert!(
                is_transient_provider_error(msg),
                "{msg:?} should be transient (infra), not a capability failure"
            );
        }

        // These are NOT transient — the model/provider rejected the request on
        // its merits. Flagging them skipped would hide real failures.
        let real_failures = [
            "400 Bad Request: invalid tool schema",
            "401 Unauthorized: invalid api key",
            "404 Not Found: model glm-5.2 does not exist",
            "the model returned a tool call with the wrong name",
        ];
        for msg in real_failures {
            assert!(
                !is_transient_provider_error(msg),
                "{msg:?} is a real failure, not transient — must count against accuracy"
            );
        }
    }

    /// Helper: a fully-passing ScenarioResult for the schema/accounting tests.
    fn pass_result(name: &str, latency_ms: u64, cost: f64) -> ScenarioResult {
        ScenarioResult {
            scenario: name.to_string(),
            parsed: true,
            correct_tool: true,
            params_present: true,
            params_correct_type: true,
            latency_ms,
            raw_output: String::new(),
            parsed_tool: Some("get_weather".to_string()),
            parsed_args: Some(json!({"location": "Tokyo"})),
            prompt_tokens: 465,
            completion_tokens: 31,
            estimated_cost_usd: cost,
            skipped: false,
        }
    }

    /// Helper: a skipped ScenarioResult (transient provider error), shaped like
    /// the real GLM-5.2 run's 8th scenario.
    fn skipped_result(name: &str, latency_ms: u64) -> ScenarioResult {
        ScenarioResult {
            scenario: name.to_string(),
            parsed: false,
            correct_tool: false,
            params_present: false,
            params_correct_type: false,
            latency_ms,
            raw_output: "SKIPPED: 429 Too Many Requests".to_string(),
            parsed_tool: None,
            parsed_args: None,
            prompt_tokens: 0,
            completion_tokens: 0,
            estimated_cost_usd: 0.0,
            skipped: true,
        }
    }
}
