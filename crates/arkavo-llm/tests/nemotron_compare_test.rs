//! Capability and speed comparison of Nemotron 3.5 Bolt against Gemma 4 12B
//! (the default desktop model) and Qwen 3.8 27B, run through Arkavo Edge's
//! own provider rather than a standalone server.
//!
//! Every request goes through `LlamaCppProvider::complete_with_tools`, so the
//! scores include our template rendering, native tool-call and reasoning
//! parsing, and how the agent loop threads tool results back. The agent
//! scenarios build each assistant turn with `as_assistant_message`, as the
//! conductor's tool loop does.
//!
//! Suites: Arkavo's seven-task check, four multi-turn agent scenarios, ten
//! coding tasks graded by hidden tests, twelve math word problems and three
//! structured extractions. Coding and the bug-fix scenario run Python, so
//! `python3` must be on PATH. Thinking is off, as the router runs these
//! models in tool loops; each model uses its vendor's recommended sampling.
//!
//! Run with: cargo test --features llama-cpp -p arkavo-llm --test nemotron_compare_test -- --ignored --nocapture
//! `NEMOTRON_BENCH_ONLY=<label substring>` picks models,
//! `NEMOTRON_BENCH_SUITES=<comma list>` picks suites,
//! `NEMOTRON_BENCH_REPS=<n>` repeats each suite (default 1), and
//! `NEMOTRON_BENCH_TRACE=1` prints every agent turn.

#![allow(clippy::disallowed_methods)] // tokio::test uses block_on internally

#[cfg(all(test, feature = "llama-cpp"))]
mod tests {
    use arkavo_llm::{
        LlamaCppProvider, LocalToolFormat, Message, Provider, ProviderResponse, Role,
        SamplingConfig, ThinkingMode,
    };
    use serde_json::{Value, json};
    use std::fmt::Write as _;
    use std::path::Path;
    use std::process::Command;
    use std::time::{Duration, Instant};

    struct Model {
        label: &'static str,
        repo_dir: &'static str,
        file: &'static str,
        temperature: f32,
        top_p: f32,
        top_k: i32,
        tool_format: LocalToolFormat,
    }

    const MODELS: &[Model] = &[
        Model {
            label: "bolt",
            repo_dir: "models--unsloth--NVIDIA-Nemotron-3.5-Bolt-74B-A9B-GGUF",
            file: "NVIDIA-Nemotron-3.5-Bolt-74B-A9B-UD-IQ2_XXS.gguf",
            temperature: 1.0,
            top_p: 0.95,
            top_k: 40,
            tool_format: LocalToolFormat::Xml,
        },
        // Gemma 4 parses its own tool-call format natively; Fence is what the
        // router and its own tests prompt it with.
        Model {
            label: "gemma-4-12b",
            repo_dir: "models--ggml-org--gemma-4-12B-it-GGUF",
            file: "gemma-4-12B-it-Q4_0.gguf",
            temperature: 0.7,
            top_p: 0.9,
            top_k: 40,
            tool_format: LocalToolFormat::Fence,
        },
        Model {
            label: "qwen3.8-27b",
            repo_dir: "models--unsloth--Qwen3.8-27B-GGUF",
            file: "Qwen3.8-27B-Q4_K_M.gguf",
            temperature: 0.7,
            top_p: 0.8,
            top_k: 20,
            tool_format: LocalToolFormat::Xml,
        },
    ];

    fn cached(repo_dir: &str, file: &str) -> Option<String> {
        let home = std::env::var("HOME").ok()?;
        std::fs::read_dir(format!(
            "{home}/.cache/huggingface/hub/{repo_dir}/snapshots"
        ))
        .ok()?
        .flatten()
        .map(|s| s.path().join(file))
        .find(|p| p.exists())
        .map(|p| p.to_string_lossy().to_string())
    }

    /// One model's results: per-suite passes and wall time, plus the
    /// provider's own decode timing and the agent loops' call latencies.
    #[derive(Default)]
    struct Card {
        suites: Vec<(&'static str, usize, usize, Duration)>,
        gen_ms: f64,
        gen_tokens: u64,
        prompt_ms: f64,
        prompt_tokens: u64,
        first_calls: Vec<f64>,
        later_calls: Vec<f64>,
        failures: Vec<String>,
        /// Replies whose content still carried template markup the parser
        /// should have removed. Kept apart from `failures`, which count
        /// failed tasks.
        leaks: Vec<String>,
    }

    impl Card {
        fn time(&mut self, r: &ProviderResponse) {
            if let Some(t) = &r.inference_timing {
                self.gen_ms += t.generation_ms;
                self.gen_tokens += u64::from(t.n_eval);
                self.prompt_ms += t.prompt_eval_ms;
                self.prompt_tokens += u64::from(t.n_prompt_eval);
            }
        }
    }

    /// Template control text that must never reach a reply's content.
    const LEAKED_MARKUP: &[&str] = &[
        "<|channel>",
        "<channel|>",
        "<think>",
        "</think>",
        "<|turn>",
        "<turn|>",
        "<|tool_call>",
        "<tool_call>",
        "<|im_end|>",
    ];

    struct Bench<'a> {
        provider: &'a LlamaCppProvider,
        card: Card,
    }

    impl Bench<'_> {
        /// A provider error (e.g. a prompt that outgrows the context window)
        /// comes back as a reply with no tool calls, so it fails the task it
        /// happened in instead of ending the whole run.
        async fn ask(&mut self, messages: Vec<Message>, tools: Option<&Value>) -> ProviderResponse {
            match self
                .provider
                .complete_with_tools(messages, tools.cloned(), None)
                .await
            {
                Ok(r) => {
                    self.card.time(&r);
                    if LEAKED_MARKUP.iter().any(|m| r.content.contains(m)) {
                        let head: String = r.content.chars().take(120).collect();
                        self.card.leaks.push(format!("{head:?}"));
                    }
                    r
                }
                Err(e) => ProviderResponse {
                    content: format!("provider error: {e}"),
                    ..Default::default()
                },
            }
        }

        fn record(&mut self, suite: &'static str, task: &str, ok: bool, detail: &str) {
            if !ok {
                let tail: String = detail
                    .chars()
                    .rev()
                    .take(140)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                self.card.failures.push(format!("{suite}/{task}: {tail:?}"));
            }
            println!("  {suite}/{task} {}", if ok { "PASS" } else { "fail" });
        }
    }

    fn user(text: &str) -> Message {
        Message {
            role: Role::User,
            content: text.to_string(),
            ..Default::default()
        }
    }

    fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
        json!({"name": name, "description": description,
               "input_schema": {"type": "object", "properties": properties, "required": required}})
    }

    fn s(description: &str) -> Value {
        json!({"type": "string", "description": description})
    }

    // ------------------------------------------------------------ arkavo7

    fn weather_tools() -> Value {
        json!([
            tool(
                "get_weather",
                "Get current weather for a city",
                json!({"location": s("City name")}),
                &["location"]
            ),
            tool(
                "get_current_time",
                "Get the current wall-clock time in a timezone or city",
                json!({"timezone": s("IANA timezone or city name")}),
                &["timezone"]
            ),
            tool(
                "calculate",
                "Evaluate an arithmetic expression",
                json!({"expression": s("Arithmetic expression, e.g. 15 * 7")}),
                &["expression"]
            ),
        ])
    }

    fn arg(r: &ProviderResponse, i: usize, key: &str) -> String {
        r.tool_calls
            .get(i)
            .and_then(|c| c.arguments.get(key))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase()
    }

    async fn arkavo7(b: &mut Bench<'_>) {
        let tools = weather_tools();
        let r = b
            .ask(vec![user("What's the weather in New York?")], Some(&tools))
            .await;
        let ok = r
            .tool_calls
            .first()
            .is_some_and(|c| c.tool_name == "get_weather")
            && arg(&r, 0, "location").contains("new york");
        b.record("arkavo7", "tool_call", ok, &r.content);

        let r = b
            .ask(
                vec![user("What time is it in Tokyo right now?")],
                Some(&tools),
            )
            .await;
        let ok = r
            .tool_calls
            .first()
            .is_some_and(|c| c.tool_name == "get_current_time")
            && arg(&r, 0, "timezone").contains("tokyo");
        b.record("arkavo7", "tool_select", ok, &r.content);

        let first = b
            .ask(vec![user("What's the weather in Paris?")], Some(&tools))
            .await;
        let ok = if let Some(call) = first
            .tool_calls
            .first()
            .filter(|c| c.tool_name == "get_weather")
        {
            let id = call.call_id.clone().unwrap_or_else(|| "call_0".to_string());
            let convo = vec![
                user("What's the weather in Paris?"),
                first.as_assistant_message(),
                Message::tool_result("Sunny, 18 degrees Celsius", &id, "get_weather"),
            ];
            let r = b.ask(convo, Some(&tools)).await;
            let a = r.content.to_lowercase();
            (a.contains("18") || a.contains("sunny")) && r.tool_calls.is_empty()
        } else {
            false
        };
        b.record("arkavo7", "multi_turn", ok, &first.content);

        let r = b
            .ask(
                vec![user(
                    "What is the capital of France? Answer in one short sentence.",
                )],
                Some(&tools),
            )
            .await;
        b.record(
            "arkavo7",
            "restraint",
            r.tool_calls.is_empty() && r.content.to_lowercase().contains("paris"),
            &r.content,
        );

        let r = b.ask(vec![user("A train travels 150 km in 2 hours, then 60 km in 1 hour. What is its average speed over the whole trip in km/h? Reply with only the number.")], None).await;
        b.record("arkavo7", "reasoning", r.content.contains("70"), &r.content);

        let r = b.ask(vec![user("Reply with exactly one lowercase word and nothing else: the plural of the word 'mouse'.")], None).await;
        let word: String = r
            .content
            .to_lowercase()
            .chars()
            .filter(char::is_ascii_lowercase)
            .collect();
        b.record("arkavo7", "instruct", word == "mice", &r.content);

        let r = b.ask(vec![user("Extract the city and temperature from this text and reply as a JSON object with keys \"city\" and \"temp\": It is 22 degrees in Berlin.")], None).await;
        let a = r.content.to_lowercase();
        b.record(
            "arkavo7",
            "extract",
            a.contains("berlin") && a.contains("22") && a.contains('{'),
            &r.content,
        );
    }

    // ------------------------------------------------------------ agent

    trait Scenario: Send {
        fn name(&self) -> &'static str;
        fn system(&self) -> String {
            "You are an agent that completes the user's task by calling tools. Finish with a short final answer.".to_string()
        }
        fn prompt(&self) -> &'static str;
        fn tools(&self) -> Value;
        fn call(&mut self, name: &str, args: &Value) -> String;
        fn judge(&self, answer: &str) -> bool;
    }

    fn arg_str<'a>(args: &'a Value, key: &str) -> &'a str {
        args.get(key).and_then(Value::as_str).unwrap_or_default()
    }

    const STATS_PY: &str = "def mean(xs):\n    return sum(xs) / (len(xs) - 1)\n\n\ndef variance(xs):\n    m = mean(xs)\n    return sum((x - m) ** 2 for x in xs) / len(xs)\n\n\ndef median(xs):\n    s = sorted(xs)\n    n = len(s)\n    mid = n // 2\n    if n % 2:\n        return s[mid]\n    return s[mid]\n";
    const TEST_PY: &str = "from stats import mean, variance, median\n\nassert mean([1, 2, 3, 4]) == 2.5, f\"mean([1,2,3,4]) = {mean([1, 2, 3, 4])}\"\nassert variance([2, 4, 4, 4, 5, 5, 7, 9]) == 4.0, \"variance\"\nassert median([3, 1, 2]) == 2, \"median odd\"\nassert median([4, 1, 3, 2]) == 2.5, f\"median([4,1,3,2]) = {median([4, 1, 3, 2])}\"\nprint(\"all tests passed\")\n";

    fn run_python(dir: &Path, script: &str) -> (bool, String) {
        match Command::new("python3")
            .args(["-E", "-s", script])
            .current_dir(dir)
            .output()
        {
            Ok(out) => {
                let text = format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
                let tail: String = text
                    .chars()
                    .rev()
                    .take(1500)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                (out.status.success(), tail)
            }
            Err(e) => (false, format!("python3 failed to start: {e}")),
        }
    }

    struct FixBug {
        dir: tempfile::TempDir,
    }

    impl FixBug {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("stats.py"), STATS_PY).unwrap();
            std::fs::write(dir.path().join("test_stats.py"), TEST_PY).unwrap();
            Self { dir }
        }
    }

    impl Scenario for FixBug {
        fn name(&self) -> &'static str {
            "fix_bug"
        }
        fn prompt(&self) -> &'static str {
            "The tests in this repository fail. Find the bugs in the code and fix them so all tests pass. Do not edit the tests."
        }
        fn tools(&self) -> Value {
            json!([
                tool("list_files", "List files in the repository", json!({}), &[]),
                tool(
                    "read_file",
                    "Read a file",
                    json!({"path": s("Relative path")}),
                    &["path"]
                ),
                tool(
                    "write_file",
                    "Replace a file's entire contents",
                    json!({"path": s("Relative path"), "content": s("Full new file contents")}),
                    &["path", "content"]
                ),
                tool(
                    "run_tests",
                    "Run the test suite and return its output",
                    json!({}),
                    &[]
                ),
            ])
        }
        fn call(&mut self, name: &str, args: &Value) -> String {
            let path = arg_str(args, "path").trim_start_matches("./").to_string();
            match name {
                "list_files" => "stats.py\ntest_stats.py".to_string(),
                "read_file" => std::fs::read_to_string(self.dir.path().join(&path))
                    .unwrap_or_else(|_| "error: no such file".into()),
                "write_file" if path.starts_with("test_") => {
                    "error: editing tests is not allowed".to_string()
                }
                "write_file" if path.contains('/') => "error: no such directory".to_string(),
                "write_file" => {
                    let content = arg_str(args, "content");
                    std::fs::write(self.dir.path().join(&path), content).unwrap();
                    format!("wrote {} bytes to {path}", content.len())
                }
                "run_tests" => run_python(self.dir.path(), "test_stats.py").1,
                _ => format!("error: unknown tool {name}"),
            }
        }
        fn judge(&self, _answer: &str) -> bool {
            let tests_intact = std::fs::read_to_string(self.dir.path().join("test_stats.py"))
                .is_ok_and(|t| t == TEST_PY);
            tests_intact && run_python(self.dir.path(), "test_stats.py").0
        }
    }

    struct Orders;

    const CUSTOMERS: &[(&str, &str)] = &[
        ("C-104", "Dana Whitfield"),
        ("C-233", "Dana Whitman"),
        ("C-310", "Dan Whitman"),
    ];
    const ORDERS: &[(&str, &str, &str, f64)] = &[
        ("C-104", "O-5001", "delivered", 88.10),
        ("C-233", "O-5102", "delivered", 42.75),
        ("C-233", "O-5140", "cancelled", 300.00),
        ("C-233", "O-5177", "delivered", 19.99),
        ("C-233", "O-5203", "shipped", 55.00),
        ("C-233", "O-5260", "delivered", 124.66),
        ("C-310", "O-5300", "delivered", 77.00),
    ];

    impl Scenario for Orders {
        fn name(&self) -> &'static str {
            "orders"
        }
        fn prompt(&self) -> &'static str {
            "How much has Dana Whitman spent in total on orders that were delivered? Give the exact dollar amount."
        }
        fn tools(&self) -> Value {
            json!([
                tool(
                    "search_customers",
                    "Search customers by name; returns matching ids and full names",
                    json!({"query": s("Name or part of a name")}),
                    &["query"]
                ),
                tool(
                    "list_orders",
                    "List a customer's order ids with their status",
                    json!({"customer_id": s("Customer id")}),
                    &["customer_id"]
                ),
                tool(
                    "get_order",
                    "Get one order's details including its total",
                    json!({"order_id": s("Order id")}),
                    &["order_id"]
                ),
            ])
        }
        fn call(&mut self, name: &str, args: &Value) -> String {
            match name {
                "search_customers" => {
                    let q = arg_str(args, "query").to_lowercase();
                    let hits: Vec<Value> = CUSTOMERS
                        .iter()
                        .filter(|(_, n)| q.split_whitespace().all(|w| n.to_lowercase().contains(w)))
                        .map(|(id, n)| json!({"id": id, "name": n}))
                        .collect();
                    Value::Array(hits).to_string()
                }
                "list_orders" => {
                    let id = arg_str(args, "customer_id");
                    if !CUSTOMERS.iter().any(|(c, _)| *c == id) {
                        return "error: unknown customer_id".to_string();
                    }
                    let rows: Vec<Value> = ORDERS
                        .iter()
                        .filter(|(c, ..)| *c == id)
                        .map(|(_, o, st, _)| json!({"order_id": o, "status": st}))
                        .collect();
                    Value::Array(rows).to_string()
                }
                "get_order" => ORDERS
                    .iter()
                    .find(|(_, o, ..)| *o == arg_str(args, "order_id"))
                    .map(|(_, o, st, t)| {
                        json!({"order_id": o, "status": st, "total": t, "currency": "USD"})
                            .to_string()
                    })
                    .unwrap_or_else(|| "error: unknown order_id".to_string()),
                _ => format!("error: unknown tool {name}"),
            }
        }
        fn judge(&self, answer: &str) -> bool {
            answer.contains("187.40") || answer.contains("187.4 ") || answer.contains("$187.4")
        }
    }

    #[derive(Default)]
    struct Schedule {
        created: Vec<(String, String, String, Vec<String>)>,
    }

    const EVENTS: &[(&str, &str, &str)] = &[
        ("09:00", "10:30", "Standup and planning"),
        ("11:00", "12:00", "1:1 with Priya"),
        ("13:00", "15:00", "Customer call"),
    ];

    impl Scenario for Schedule {
        fn name(&self) -> &'static str {
            "schedule"
        }
        fn system(&self) -> String {
            "You are an agent that completes the user's task by calling tools. Finish with a short final answer. Today is Thursday 2026-10-15. Working hours are 08:00-18:00.".to_string()
        }
        fn prompt(&self) -> &'static str {
            "Book a one-hour 'Design review' meeting with Sam Okafor next Tuesday. Morning if at all possible, and it must not overlap anything already on my calendar."
        }
        fn tools(&self) -> Value {
            json!([
                tool(
                    "get_calendar",
                    "Get the events on a date",
                    json!({"date": s("Date as YYYY-MM-DD")}),
                    &["date"]
                ),
                tool(
                    "find_contact",
                    "Look up a contact's email by name",
                    json!({"name": s("Person's name")}),
                    &["name"]
                ),
                tool(
                    "create_event",
                    "Create a calendar event",
                    json!({"title": s("Title"), "date": s("YYYY-MM-DD"), "start": s("HH:MM 24h"), "end": s("HH:MM 24h"),
                            "attendees": {"type": "array", "items": {"type": "string"}, "description": "Attendee emails"}}),
                    &["title", "date", "start", "end", "attendees"]
                ),
            ])
        }
        fn call(&mut self, name: &str, args: &Value) -> String {
            let date = arg_str(args, "date");
            let iso = date.len() == 10 && date.as_bytes()[4] == b'-' && date.as_bytes()[7] == b'-';
            match name {
                "get_calendar" if !iso => "error: date must be YYYY-MM-DD".to_string(),
                "get_calendar" if date != "2026-10-20" => {
                    json!({"date": date, "events": []}).to_string()
                }
                "get_calendar" => {
                    let events: Vec<Value> = EVENTS
                        .iter()
                        .map(|(a, b, t)| json!({"start": a, "end": b, "title": t}))
                        .collect();
                    json!({"date": date, "events": events}).to_string()
                }
                "find_contact" if arg_str(args, "name").to_lowercase().contains("sam") => {
                    json!({"name": "Sam Okafor", "email": "sam.okafor@example.com"}).to_string()
                }
                "find_contact" => "error: no contact found".to_string(),
                "create_event" => {
                    let attendees: Vec<String> = match args.get("attendees") {
                        Some(Value::Array(a)) => a
                            .iter()
                            .filter_map(Value::as_str)
                            .map(String::from)
                            .collect(),
                        Some(Value::String(one)) => vec![one.clone()],
                        _ => Vec::new(),
                    };
                    if attendees.is_empty()
                        || !attendees
                            .iter()
                            .all(|a| a.contains('@') && !a.contains(' '))
                    {
                        return "error: attendees must be email addresses".to_string();
                    }
                    let (start, end) = (arg_str(args, "start"), arg_str(args, "end"));
                    if date == "2026-10-20"
                        && let Some((a, b, t)) =
                            EVENTS.iter().find(|(a, b, _)| start < *b && *a < end)
                    {
                        return format!("error: overlaps '{t}' ({a}-{b})");
                    }
                    self.created.push((
                        date.to_string(),
                        start.to_string(),
                        end.to_string(),
                        attendees,
                    ));
                    json!({"status": "created", "event_id": format!("E-{}", self.created.len())})
                        .to_string()
                }
                _ => format!("error: unknown tool {name}"),
            }
        }
        fn judge(&self, _answer: &str) -> bool {
            self.created
                == vec![(
                    "2026-10-20".to_string(),
                    "08:00".to_string(),
                    "09:00".to_string(),
                    vec!["sam.okafor@example.com".to_string()],
                )]
        }
    }

    struct LogNeedle {
        lines: Vec<String>,
    }

    impl LogNeedle {
        fn new() -> Self {
            let lines = (0..300)
                .map(|i| {
                    let ms = match i {
                        217 => 2950,
                        54 => 1980,
                        _ => (i * 37) % 400 + 20,
                    };
                    format!("2026-10-15T10:{:02}:{:02}Z req=r{} path=/api/v1/items status=200 latency_ms={ms}", i / 60, i % 60, 1000 + i)
                })
                .collect();
            Self { lines }
        }
    }

    impl Scenario for LogNeedle {
        fn name(&self) -> &'static str {
            "log_needle"
        }
        fn prompt(&self) -> &'static str {
            "Which request id in app.log had the highest latency, and what was it? The file is long."
        }
        fn tools(&self) -> Value {
            json!([tool(
                "read_file",
                "Read up to 100 lines of a file starting at a 0-based line offset",
                json!({"path": s("Relative path"),
                               "offset": {"type": "integer", "description": "First line, 0-based"},
                               "limit": {"type": "integer", "description": "Lines to read, at most 100"}}),
                &["path"]
            )])
        }
        fn call(&mut self, name: &str, args: &Value) -> String {
            if name != "read_file" || arg_str(args, "path").trim_start_matches("./") != "app.log" {
                return "error: no such file".to_string();
            }
            let num = |k: &str, d: usize| {
                args.get(k)
                    .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
                    .map_or(d, |n| n as usize)
            };
            let off = num("offset", 0).min(self.lines.len());
            let end = (off + num("limit", 100).min(100)).min(self.lines.len());
            let mut out = self.lines[off..end].join("\n");
            if end < self.lines.len() {
                let _ = write!(
                    out,
                    "\n[{} more lines; continue at offset {end}]",
                    self.lines.len() - end
                );
            } else {
                out.push_str("\n[end of file]");
            }
            out
        }
        fn judge(&self, answer: &str) -> bool {
            answer.contains("r1217") && answer.contains("2950")
        }
    }

    async fn agent(b: &mut Bench<'_>) {
        let scenarios: Vec<Box<dyn Scenario>> = vec![
            Box::new(FixBug::new()),
            Box::new(Orders),
            Box::new(Schedule::default()),
            Box::new(LogNeedle::new()),
        ];
        let trace = std::env::var_os("NEMOTRON_BENCH_TRACE").is_some();
        for mut sc in scenarios {
            let tools = sc.tools();
            let mut messages = vec![Message::system(sc.system()), user(sc.prompt())];
            let mut answer = String::new();
            let mut turns = 0;
            for turn in 0..15 {
                turns = turn + 1;
                let t0 = Instant::now();
                let r = b.ask(messages.clone(), Some(&tools)).await;
                let secs = t0.elapsed().as_secs_f64();
                if turn == 0 {
                    b.card.first_calls.push(secs);
                } else {
                    b.card.later_calls.push(secs);
                }
                if trace {
                    println!(
                        "    [{}#{turn} {secs:.1}s] content={:?}",
                        sc.name(),
                        r.content.chars().take(200).collect::<String>()
                    );
                }
                if r.tool_calls.is_empty() {
                    answer = r.content;
                    break;
                }
                messages.push(r.as_assistant_message());
                for (i, call) in r.tool_calls.iter().enumerate() {
                    let id = call
                        .call_id
                        .clone()
                        .unwrap_or_else(|| format!("call_{turn}_{i}"));
                    let result = sc.call(&call.tool_name, &call.arguments);
                    if trace {
                        let args: String = call.arguments.to_string().chars().take(300).collect();
                        let head: String = result.chars().take(200).collect();
                        println!("      call {}({args}) -> {head:?}", call.tool_name);
                    }
                    messages.push(Message::tool_result(result, &id, &call.tool_name));
                }
            }
            let ok = sc.judge(&answer);
            b.record("agent", sc.name(), ok, &format!("{turns} turns: {answer}"));
        }
    }

    // ------------------------------------------------------------ code

    const PYTEST_STUB: &str = "def raises(exc, f, *a):\n    try:\n        f(*a)\n    except exc:\n        return\n    raise AssertionError(f\"{f.__name__}{a} did not raise {exc.__name__}\")\n";

    const CODE: &[(&str, &str, &[&str])] = &[
        (
            "merge_intervals",
            "merge_intervals(intervals: list[list[int]]) -> list[list[int]]: merge overlapping closed intervals (touching ones like [1,2],[2,3] merge too); input may be unsorted; return sorted by start.",
            &[
                "assert merge_intervals([[1,3],[2,6],[8,10],[15,18]]) == [[1,6],[8,10],[15,18]]",
                "assert merge_intervals([[1,4],[4,5]]) == [[1,5]]",
                "assert merge_intervals([[5,7],[1,2]]) == [[1,2],[5,7]]",
                "assert merge_intervals([]) == []",
                "assert merge_intervals([[1,10],[2,3],[4,5]]) == [[1,10]]",
            ],
        ),
        (
            "int_to_roman",
            "int_to_roman(n: int) -> str: convert 1..3999 to a Roman numeral using subtractive notation.",
            &[
                "assert int_to_roman(3) == 'III'",
                "assert int_to_roman(58) == 'LVIII'",
                "assert int_to_roman(1994) == 'MCMXCIV'",
                "assert int_to_roman(3999) == 'MMMCMXCIX'",
                "assert int_to_roman(444) == 'CDXLIV'",
            ],
        ),
        (
            "eval_rpn",
            "eval_rpn(tokens: list[str]) -> int: evaluate Reverse Polish Notation with + - * / on integers; division truncates toward zero.",
            &[
                "assert eval_rpn(['2','1','+','3','*']) == 9",
                "assert eval_rpn(['4','13','5','/','+']) == 6",
                "assert eval_rpn(['10','6','9','3','+','-11','*','/','*','17','+','5','+']) == 22",
                "assert eval_rpn(['7','-2','/']) == -3",
            ],
        ),
        (
            "top_k_frequent",
            "top_k_frequent(words: list[str], k: int) -> list[str]: the k most frequent words, ordered by frequency descending, ties broken alphabetically.",
            &[
                "assert top_k_frequent(['i','love','leetcode','i','love','coding'], 2) == ['i','love']",
                "assert top_k_frequent(['the','day','is','sunny','the','the','the','sunny','is','is'], 4) == ['the','is','sunny','day']",
                "assert top_k_frequent(['b','a','c'], 2) == ['a','b']",
            ],
        ),
        (
            "parse_duration",
            "parse_duration(s: str) -> int: parse durations like '1h30m15s', '2d', '45m', '1d2h' into total seconds. Units d,h,m,s each appear at most once, in that order. Raise ValueError for anything else (empty string, unknown unit, wrong order, missing number).",
            &[
                "assert parse_duration('1h30m15s') == 5415",
                "assert parse_duration('2d') == 172800",
                "assert parse_duration('45m') == 2700",
                "assert parse_duration('1d2h') == 93600",
                "import pytest_stub as _p; _p.raises(ValueError, parse_duration, '')",
                "import pytest_stub as _p; _p.raises(ValueError, parse_duration, '5x')",
                "import pytest_stub as _p; _p.raises(ValueError, parse_duration, '30m1h')",
                "import pytest_stub as _p; _p.raises(ValueError, parse_duration, 'h')",
            ],
        ),
        (
            "topo_sort",
            "topo_sort(n: int, edges: list[tuple[int,int]]) -> list[int] | None: nodes 0..n-1, edge (a,b) means a before b. Return the lexicographically smallest topological order, or None if there is a cycle.",
            &[
                "assert topo_sort(4, [(1,0),(2,0),(3,1)]) == [2,3,1,0]",
                "assert topo_sort(3, []) == [0,1,2]",
                "assert topo_sort(2, [(0,1),(1,0)]) is None",
                "assert topo_sort(5, [(4,0),(3,0),(0,1)]) == [2,3,4,0,1]",
            ],
        ),
        (
            "min_window",
            "min_window(s: str, t: str) -> str: the shortest substring of s containing every character of t with multiplicity; '' if none; if several, the leftmost.",
            &[
                "assert min_window('ADOBECODEBANC', 'ABC') == 'BANC'",
                "assert min_window('a', 'aa') == ''",
                "assert min_window('aa', 'aa') == 'aa'",
                "assert min_window('abcabdebac', 'cda') == 'cabd'",
            ],
        ),
        (
            "spiral",
            "spiral(m: list[list[int]]) -> list[int]: elements of a rectangular matrix in clockwise spiral order.",
            &[
                "assert spiral([[1,2,3],[4,5,6],[7,8,9]]) == [1,2,3,6,9,8,7,4,5]",
                "assert spiral([[1,2,3,4],[5,6,7,8],[9,10,11,12]]) == [1,2,3,4,8,12,11,10,9,5,6,7]",
                "assert spiral([[1],[2],[3]]) == [1,2,3]",
                "assert spiral([]) == []",
            ],
        ),
        (
            "next_permutation",
            "next_permutation(nums: list[int]) -> list[int]: the next lexicographic permutation, wrapping to the smallest when nums is the largest. Return a new list.",
            &[
                "assert next_permutation([1,2,3]) == [1,3,2]",
                "assert next_permutation([3,2,1]) == [1,2,3]",
                "assert next_permutation([1,1,5]) == [1,5,1]",
                "assert next_permutation([1,3,2]) == [2,1,3]",
            ],
        ),
        (
            "wildcard_match",
            "wildcard_match(s: str, p: str) -> bool: '?' matches one character, '*' any sequence including empty; the whole string must match.",
            &[
                "assert wildcard_match('aa', 'a') is False",
                "assert wildcard_match('aa', '*') is True",
                "assert wildcard_match('cb', '?a') is False",
                "assert wildcard_match('adceb', '*a*b') is True",
                "assert wildcard_match('acdcb', 'a*c?b') is False",
                "assert wildcard_match('', '***') is True",
            ],
        ),
    ];

    /// The longest fenced Python block, or the whole reply when there is none.
    fn extract_code(text: &str) -> String {
        let re = regex::Regex::new(r"(?s)```(?:python|py)?\s*\n(.*?)```").unwrap();
        re.captures_iter(text)
            .map(|c| c[1].to_string())
            .max_by_key(String::len)
            .unwrap_or_else(|| text.to_string())
    }

    async fn code(b: &mut Bench<'_>) {
        for (name, spec, tests) in CODE {
            let r = b
                .ask(vec![user(&format!("Write a Python function {spec}\nUse only the standard library. Reply with the complete function in one ```python code block."))], None)
                .await;
            let dir = tempfile::tempdir().unwrap();
            let program = format!(
                "{}\n\n{}\nprint('ok')\n",
                extract_code(&r.content),
                tests.join("\n")
            );
            std::fs::write(dir.path().join("sol.py"), program).unwrap();
            std::fs::write(dir.path().join("pytest_stub.py"), PYTEST_STUB).unwrap();
            let (ok, out) = run_python(dir.path(), "sol.py");
            b.record("code", name, ok && out.contains("ok"), &out);
        }
    }

    // ------------------------------------------------------------ math

    const MATH: &[(&str, &str, f64)] = &[
        (
            "pens",
            "Pens cost $4 for a pack of 3 (packs only) and notebooks cost $2.50 each. Maria buys 18 pens and 7 notebooks and pays with a $50 bill. How much change does she get, in dollars?",
            8.5,
        ),
        (
            "div35",
            "How many integers from 1 to 1000 are divisible by 3 or by 5, but not by 15?",
            401.0,
        ),
        (
            "pipes",
            "Pipe A fills a tank in 6 hours, pipe B fills it in 4 hours, and drain C empties a full tank in 12 hours. Starting empty with all three open, how many hours until the tank is full?",
            3.0,
        ),
        (
            "digits9",
            "What is the sum of all two-digit positive integers whose digits add up to 9?",
            486.0,
        ),
        (
            "interest",
            "$2000 is invested at 5% interest compounded annually. How many dollars of interest have been earned after 3 years? Round to the cent.",
            315.25,
        ),
        (
            "balloon",
            "How many distinct arrangements are there of the letters in the word BALLOON?",
            1260.0,
        ),
        (
            "clock",
            "What is the smaller angle, in degrees, between the hour and minute hands of a clock at 3:40?",
            130.0,
        ),
        (
            "dice",
            "Two fair six-sided dice are rolled. What is the probability that the product of the numbers is even? Give a decimal.",
            0.75,
        ),
        (
            "bird",
            "Two trains 300 km apart travel toward each other at 70 km/h and 80 km/h. A bird flies at 120 km/h back and forth between them until they meet. How many km does the bird fly?",
            240.0,
        ),
        (
            "pow7",
            "What are the last two digits of 7^2026? Give them as a number.",
            49.0,
        ),
        ("zeros", "How many trailing zeros does 125! have?", 31.0),
        (
            "walls",
            "5 workers build 5 walls in 5 days. Working at the same rate, how many days do 10 workers need to build 20 walls?",
            10.0,
        ),
    ];

    /// The number after `ANSWER:`, or the last number in the reply.
    fn final_number(text: &str) -> Option<f64> {
        let tagged = regex::Regex::new(r"ANSWER:\s*\$?\s*(-?[\d,]*\.?\d+)").unwrap();
        let any = regex::Regex::new(r"-?\d[\d,]*\.?\d*").unwrap();
        let raw = match tagged.captures(text) {
            Some(c) => c[1].to_string(),
            None => any
                .find_iter(text)
                .last()?
                .as_str()
                .trim_end_matches('.')
                .to_string(),
        };
        raw.replace(',', "").parse().ok()
    }

    async fn math(b: &mut Bench<'_>) {
        for (name, question, answer) in MATH {
            let r = b
                .ask(
                    vec![user(&format!(
                        "{question}\nEnd your reply with a final line 'ANSWER: <number>'."
                    ))],
                    None,
                )
                .await;
            let got = final_number(&r.content);
            b.record(
                "math",
                name,
                got.is_some_and(|g| (g - answer).abs() < 1e-6),
                &format!("{got:?}"),
            );
        }
    }

    // ------------------------------------------------------------ json

    const INVOICE: &str = "INVOICE  #INV-2026-0912     Northwind Traders Ltd.\nIssued: October 3rd, 2026        Due in 30 days\n---------------------------------------------\n2 x Ergonomic keyboard ........ @ 89.50\n1 x 27in monitor .............. @ 329.00\n4 x USB-C cable (2m) .......... @ 12.25\nShipping: free     Tax: included\nTOTAL DUE: 557.00";
    const TICKETS: &str = "1. \"I was charged twice for my October subscription.\"\n2. \"The app crashes every time I open the settings page on Android 15.\"\n3. \"How do I export my data to CSV?\"\n4. \"Someone logged into my account from another country and changed my password!\"\n5. \"Feature idea: dark mode for the dashboard.\"";
    const NOTES: &str = "Sync 2026-10-14 - attendees: Priya, Marco, Lena.\nWe agreed to ship v2.3 on Oct 28 and to drop IE11 support. Marco will finish the migration script by 2026-10-21.\nLena owns updating the docs, due Oct 24 2026. Priya to send the release announcement the day of the ship.";

    /// The first JSON value in a reply, inside a fence or not.
    fn parse_json(text: &str) -> Option<Value> {
        let fenced = regex::Regex::new(r"(?s)```(?:json)?\s*\n(.*?)```").unwrap();
        let body = fenced
            .captures(text)
            .map_or_else(|| text.to_string(), |c| c[1].to_string());
        let start = body.find(['{', '['])?;
        serde_json::Deserializer::from_str(&body[start..])
            .into_iter::<Value>()
            .next()?
            .ok()
    }

    fn num(v: &Value) -> f64 {
        v.as_f64()
            .or_else(|| v.as_str()?.parse().ok())
            .unwrap_or(f64::NAN)
    }

    async fn json_suite(b: &mut Bench<'_>) {
        let r = b.ask(vec![user(&format!("Extract this invoice as JSON with exactly these keys: vendor (string), invoice_number (string), date (YYYY-MM-DD), total (number), line_items (array of objects with description (string), qty (integer), unit_price (number)). Reply with JSON only.\n\n{INVOICE}"))], None).await;
        let ok = parse_json(&r.content).is_some_and(|j| {
            let mut items: Vec<(i64, f64)> = j["line_items"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|i| (num(&i["qty"]) as i64, num(&i["unit_price"])))
                        .collect()
                })
                .unwrap_or_default();
            items.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            j["invoice_number"] == "INV-2026-0912"
                && j["date"] == "2026-10-03"
                && (num(&j["total"]) - 557.0).abs() < 0.005
                && j["vendor"]
                    .as_str()
                    .is_some_and(|v| v.to_lowercase().contains("northwind"))
                && items == vec![(1, 329.0), (2, 89.5), (4, 12.25)]
        });
        b.record("json", "invoice", ok, &r.content);

        let r = b.ask(vec![user(&format!("Classify each support ticket. Reply with a JSON array of objects {{\"id\": integer, \"category\": one of \"billing\"|\"bug\"|\"how_to\"|\"security\"|\"feature_request\", \"priority\": one of \"low\"|\"medium\"|\"high\"}}. Security incidents are always high priority. Reply with JSON only.\n\n{TICKETS}"))], None).await;
        let ok = parse_json(&r.content)
            .and_then(|j| j.as_array().cloned())
            .is_some_and(|rows| {
                let want = [
                    (1, "billing"),
                    (2, "bug"),
                    (3, "how_to"),
                    (4, "security"),
                    (5, "feature_request"),
                ];
                rows.len() == 5
                    && want.iter().all(|(id, cat)| {
                        rows.iter()
                            .any(|r| num(&r["id"]) as i64 == *id && r["category"] == *cat)
                    })
                    && rows
                        .iter()
                        .any(|r| num(&r["id"]) as i64 == 4 && r["priority"] == "high")
            });
        b.record("json", "tickets", ok, &r.content);

        let r = b.ask(vec![user(&format!("Turn these meeting notes into JSON with keys attendees (array of names), decisions (array of strings), action_items (array of {{owner, task, due}} with due as YYYY-MM-DD). Reply with JSON only.\n\n{NOTES}"))], None).await;
        let ok = parse_json(&r.content).is_some_and(|j| {
            let mut attendees: Vec<&str> = j["attendees"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            attendees.sort_unstable();
            let due = |owner: &str| {
                j["action_items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|a| a["owner"].as_str().is_some_and(|o| o.starts_with(owner)))
                    .and_then(|a| a["due"].as_str())
            };
            attendees == ["Lena", "Marco", "Priya"]
                && j["decisions"].as_array().is_some_and(|d| d.len() >= 2)
                && due("Marco") == Some("2026-10-21")
                && due("Lena") == Some("2026-10-24")
                && due("Priya") == Some("2026-10-28")
        });
        b.record("json", "notes", ok, &r.content);
    }

    // ------------------------------------------------------------ driver

    #[tokio::test]
    #[ignore = "benchmark: loads Nemotron 3.5 Bolt, Gemma 4 12B and Qwen 3.8 27B (~46GB cached), most of an hour"]
    async fn compare_nemotron_bolt_with_gemma4_and_qwen38() {
        let only = std::env::var("NEMOTRON_BENCH_ONLY").ok();
        let reps: usize = std::env::var("NEMOTRON_BENCH_REPS")
            .ok()
            .and_then(|r| r.parse().ok())
            .unwrap_or(1);
        let mut cards: Vec<(&str, f64, Card)> = Vec::new();

        for m in MODELS {
            if only.as_deref().is_some_and(|o| !m.label.contains(o)) {
                continue;
            }
            let Some(path) = cached(m.repo_dir, m.file) else {
                eprintln!("SKIP {}: {} not cached", m.label, m.file);
                continue;
            };
            println!("\n== {}", m.label);
            let config = SamplingConfig {
                temperature: m.temperature,
                top_p: m.top_p,
                top_k: m.top_k,
                tool_format: m.tool_format,
                thinking_mode: Some(ThinkingMode::Off),
                max_tokens: 3072,
                ..Default::default()
            };
            let load = Instant::now();
            let provider =
                LlamaCppProvider::new_with_config(m.label.to_string(), path, None, config)
                    .expect("load failed");
            let load_s = load.elapsed().as_secs_f64();

            let mut b = Bench {
                provider: &provider,
                card: Card::default(),
            };
            let suites = std::env::var("NEMOTRON_BENCH_SUITES")
                .unwrap_or_else(|_| "arkavo7,agent,code,math,json".to_string());
            for suite in ["arkavo7", "agent", "code", "math", "json"] {
                if !suites.split(',').any(|s| s == suite) {
                    continue;
                }
                let before = b.card.failures.len();
                let t0 = Instant::now();
                let mut total = 0;
                for _ in 0..reps {
                    match suite {
                        "arkavo7" => {
                            arkavo7(&mut b).await;
                            total += 7;
                        }
                        "agent" => {
                            agent(&mut b).await;
                            total += 4;
                        }
                        "code" => {
                            code(&mut b).await;
                            total += CODE.len();
                        }
                        "math" => {
                            math(&mut b).await;
                            total += MATH.len();
                        }
                        _ => {
                            json_suite(&mut b).await;
                            total += 3;
                        }
                    }
                }
                let failed = b.card.failures.len() - before;
                b.card
                    .suites
                    .push((suite, total - failed, total, t0.elapsed()));
            }
            cards.push((m.label, load_s, b.card));
        }

        let median = |v: &[f64]| {
            let mut v = v.to_vec();
            v.sort_by(|a, b| a.total_cmp(b));
            v.get(v.len() / 2).copied().unwrap_or(0.0)
        };
        println!(
            "\n========= NEMOTRON 3.5 BOLT vs GEMMA 4 12B vs QWEN 3.8 (through LlamaCppProvider) ========="
        );
        println!(
            "{:<14}{:>10}{:>10}{:>10}{:>10}{:>10}{:>9}{:>9}{:>8}{:>9}{:>9}{:>8}",
            "model",
            "arkavo7",
            "agent",
            "code",
            "math",
            "json",
            "total",
            "wall_s",
            "load_s",
            "pp t/s",
            "tg t/s",
            "leaks"
        );
        for (label, load_s, card) in &cards {
            print!("{label:<14}");
            let (mut p, mut n, mut wall) = (0, 0, 0.0);
            for name in ["arkavo7", "agent", "code", "math", "json"] {
                match card.suites.iter().find(|(s, ..)| *s == name) {
                    Some((_, passed, total, t)) => {
                        print!("{:>10}", format!("{passed}/{total}"));
                        p += passed;
                        n += total;
                        wall += t.as_secs_f64();
                    }
                    None => print!("{:>10}", "-"),
                }
            }
            // A provider that reports no decode time gets "-", not a
            // division by zero.
            let rate = |tokens: u64, ms: f64, prec: usize| {
                if ms > 0.0 {
                    format!("{:.prec$}", tokens as f64 / (ms / 1000.0))
                } else {
                    "-".to_string()
                }
            };
            println!(
                "{:>9}{wall:>9.0}{load_s:>8.1}{:>9}{:>9}{:>8}",
                format!("{p}/{n}"),
                rate(card.prompt_tokens, card.prompt_ms, 0),
                rate(card.gen_tokens, card.gen_ms, 1),
                card.leaks.len()
            );
        }
        println!("\nagent loop latency, median seconds: first call / every later call");
        for (label, _, card) in &cards {
            println!(
                "  {label:<14}{:>6.2} / {:.2}",
                median(&card.first_calls),
                median(&card.later_calls)
            );
        }
        println!("\nfailures");
        for (label, _, card) in &cards {
            for f in &card.failures {
                println!("  {label}: {f}");
            }
        }
        println!("\nmarkup left in replies");
        for (label, _, card) in &cards {
            for l in &card.leaks {
                println!("  {label}: {l}");
            }
        }
        println!(
            "thinking off, {reps} rep(s); vendor-recommended sampling; pp/tg from the provider's own timing"
        );

        assert!(!cards.is_empty(), "no models were benchmarked");
    }
}
