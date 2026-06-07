//! Capability comparison across the Gemma 4 models Arkavo Edge supports.
//!
//! Probes the dimensions that matter for an agentic CLI — tool calling, tool
//! selection, multi-turn grounding, tool restraint, reasoning, instruction
//! following, structured extraction — with deterministic auto-scoring (seed
//! 42, temp 0.1) plus wall-clock speed. Prints a scorecard to inform the
//! default install model choice.
//!
//! Single-shot per task: results are a reproducible snapshot, not a
//! robustness average. The multi_turn task depends on the provider's Gemma 4
//! tool-call synthesis (empty `{}` args), which is sensitive to small prompt
//! changes — treat it as a weak signal.
//!
//! Run with: cargo test --features llama-cpp -p arkavo-llm --test gemma4_compare_test -- --ignored --nocapture

#![allow(clippy::disallowed_methods)]

#[cfg(all(test, feature = "llama-cpp"))]
mod tests {
    use arkavo_llm::{
        LlamaCppProvider, LocalToolFormat, Message, Provider, ProviderResponse, Role,
        SamplingConfig,
    };
    use std::time::{Duration, Instant};

    /// (label, approx size, HF repo, GGUF filename) for each supported Gemma 4.
    const MODELS: &[(&str, &str, &str, &str)] = &[
        (
            "E2B (2.3B act)",
            "2.9G",
            "unsloth/gemma-4-E2B-it-GGUF",
            "gemma-4-E2B-it-Q4_K_M.gguf",
        ),
        (
            "E4B (4.5B act)",
            "5.0G",
            "ggml-org/gemma-4-E4B-it-GGUF",
            "gemma-4-e4b-it-Q4_K_M.gguf",
        ),
        (
            "12B dense",
            "6.9G",
            "ggml-org/gemma-4-12B-it-GGUF",
            "gemma-4-12B-it-Q4_K_M.gguf",
        ),
        (
            "26B-A4B MoE",
            "16G",
            "ggml-org/gemma-4-26B-A4B-it-GGUF",
            "gemma-4-26B-A4B-it-Q4_K_M.gguf",
        ),
        (
            "31B dense",
            "17G",
            "ggml-org/gemma-4-31B-it-GGUF",
            "gemma-4-31B-it-Q4_K_M.gguf",
        ),
    ];

    const TOOLS: &str = r#"[
        {"name":"get_weather","description":"Get current weather for a city","input_schema":{"type":"object","properties":{"location":{"type":"string","description":"City name"}},"required":["location"]}},
        {"name":"get_current_time","description":"Get the current wall-clock time in a timezone or city","input_schema":{"type":"object","properties":{"timezone":{"type":"string","description":"IANA timezone or city name"}},"required":["timezone"]}},
        {"name":"calculate","description":"Evaluate an arithmetic expression","input_schema":{"type":"object","properties":{"expression":{"type":"string","description":"Arithmetic expression, e.g. 15 * 7"}},"required":["expression"]}}
    ]"#;

    fn find_model(repo: &str, file: &str) -> Option<String> {
        let home = std::env::var("HOME").ok()?;
        let dir = format!(
            "{home}/.cache/huggingface/hub/models--{}/snapshots",
            repo.replace('/', "--")
        );
        for snap in std::fs::read_dir(dir).ok()?.flatten() {
            let p = snap.path().join(file);
            if p.exists() {
                return Some(p.to_string_lossy().to_string());
            }
        }
        None
    }

    fn user(content: &str) -> Message {
        Message {
            role: Role::User,
            content: content.to_string(),
            images: None,
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        }
    }

    /// One scored call: returns (response, wall-clock elapsed).
    async fn ask(
        p: &LlamaCppProvider,
        msgs: Vec<Message>,
        tools: Option<&str>,
        elapsed: &mut Duration,
    ) -> ProviderResponse {
        let tv = tools.map(|t| serde_json::from_str::<serde_json::Value>(t).unwrap());
        let t0 = Instant::now();
        let r = p
            .complete_with_tools(msgs, tv, None)
            .await
            .expect("completion failed");
        *elapsed += t0.elapsed();
        r
    }

    #[tokio::test]
    #[ignore = "benchmark: loads all 5 Gemma 4 models (~48GB cached), several minutes"]
    async fn compare_gemma4_models() {
        let task_names = [
            "tool_call",
            "tool_select",
            "multi_turn",
            "restraint",
            "reasoning",
            "instruct",
            "extract",
        ];
        // label, size, passes, infer_s, load_s
        let mut scorecard: Vec<(String, String, Vec<bool>, f64, f64)> = Vec::new();
        let only = std::env::var("GEMMA4_BENCH_ONLY").ok();

        for (label, size, repo, file) in MODELS {
            if only.as_ref().is_some_and(|f| !label.contains(f.as_str())) {
                continue;
            }
            let path = match find_model(repo, file) {
                Some(p) => p,
                None => {
                    eprintln!("SKIP {label}: {repo}/{file} not cached");
                    continue;
                }
            };
            let config = SamplingConfig {
                temperature: 0.1,
                tool_format: LocalToolFormat::Fence,
                max_tokens: 512,
                debug: only.is_some(),
                ..Default::default()
            };
            let load_start = Instant::now();
            let p = LlamaCppProvider::new_with_config(label.to_string(), path, None, config)
                .expect("load failed");
            let load_s = load_start.elapsed().as_secs_f64();

            let mut passes: Vec<bool> = Vec::new();
            let mut infer = Duration::ZERO;

            // 1. Single tool call.
            let r = ask(
                &p,
                vec![user("What's the weather in New York?")],
                Some(TOOLS),
                &mut infer,
            )
            .await;
            passes.push(r.tool_calls.iter().any(|c| {
                c.tool_name == "get_weather"
                    && c.arguments
                        .get("location")
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| s.to_lowercase().contains("new york"))
            }));

            // 2. Tool selection among three tools.
            let r = ask(
                &p,
                vec![user("What time is it in Tokyo right now?")],
                Some(TOOLS),
                &mut infer,
            )
            .await;
            passes.push(r.tool_calls.first().is_some_and(|c| {
                c.tool_name == "get_current_time"
                    && c.arguments
                        .get("timezone")
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| s.to_lowercase().contains("tokyo"))
            }));

            // 3. Multi-turn grounding (weak signal: empty-args synthesis).
            let r0 = ask(
                &p,
                vec![user("What's the weather in Paris?")],
                Some(TOOLS),
                &mut infer,
            )
            .await;
            let mt = if let Some(c) = r0
                .tool_calls
                .first()
                .filter(|c| c.tool_name == "get_weather")
            {
                let id = c.call_id.clone().unwrap_or_else(|| "call_0".to_string());
                let convo = vec![
                    user("What's the weather in Paris?"),
                    Message::assistant(&r0.content),
                    Message::tool_result("Sunny, 18 degrees Celsius", &id, "get_weather"),
                ];
                let r1 = ask(&p, convo, Some(TOOLS), &mut infer).await;
                let a = r1.content.to_lowercase();
                (a.contains("18") || a.contains("sunny")) && r1.tool_calls.is_empty()
            } else {
                false
            };
            passes.push(mt);

            // 4. Tool restraint: tools available but none needed.
            let r = ask(
                &p,
                vec![user(
                    "What is the capital of France? Answer in one short sentence.",
                )],
                Some(TOOLS),
                &mut infer,
            )
            .await;
            passes.push(r.tool_calls.is_empty() && r.content.to_lowercase().contains("paris"));

            // 5. Multi-step reasoning (no tools).
            let r = ask(&p, vec![user("A train travels 150 km in 2 hours, then 60 km in 1 hour. What is its average speed over the whole trip in km/h? Reply with only the number.")], None, &mut infer).await;
            passes.push(r.content.contains("70"));

            // 6. Instruction following (exact output).
            let r = ask(&p, vec![user("Reply with exactly one lowercase word and nothing else: the plural of the word 'mouse'.")], None, &mut infer).await;
            let ans: String = r
                .content
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphabetic())
                .collect();
            passes.push(ans == "mice");

            // 7. Structured extraction.
            let r = ask(&p, vec![user("Extract the city and temperature from this text and reply as a JSON object with keys \"city\" and \"temp\": It is 22 degrees in Berlin.")], None, &mut infer).await;
            let a = r.content.to_lowercase();
            passes.push(a.contains("berlin") && a.contains("22") && a.contains('{'));

            scorecard.push((
                label.to_string(),
                size.to_string(),
                passes,
                infer.as_secs_f64(),
                load_s,
            ));
        }

        println!("\n================== GEMMA 4 CAPABILITY SCORECARD ==================");
        print!("{:<16}{:>5}", "model", "size");
        for n in &task_names {
            print!("{n:>10}");
        }
        println!("{:>7}{:>9}{:>9}", "score", "infer_s", "load_s");
        for (label, size, passes, infer_s, load_s) in &scorecard {
            print!("{label:<16}{size:>5}");
            for p in passes {
                print!("{:>10}", if *p { "PASS" } else { "·" });
            }
            let score = passes.iter().filter(|b| **b).count();
            println!(
                "{:>5}/{}{:>9.1}{:>9.1}",
                score,
                passes.len(),
                infer_s,
                load_s
            );
        }
        println!("=================================================================");
        println!(
            "single-shot, seed 42, temp 0.1. tasks: {}",
            task_names.join(", ")
        );

        assert!(!scorecard.is_empty(), "no models were benchmarked");
    }
}
