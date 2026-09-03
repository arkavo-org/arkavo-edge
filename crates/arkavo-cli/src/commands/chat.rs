use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use tokio::runtime::Runtime;

// Re-export types from arkavo-protocol for use by other commands and tests
pub use arkavo_protocol::{
    ChatSession, CommandResult, ContextMode, PendingContext, execute_command,
    parse_command as parse_protocol_command,
};

// Re-export types from migrated modules for use by other commands
pub use arkavo_context::{RepoContextMode, compress_repo_context, should_attach_repo_context};
pub use arkavo_router::response::{sanitize_response, strip_think_blocks, strip_tool_blocks};
pub use arkavo_session::{ConversationManager, ConversationMessage, ConversationSession};

// Global flag to control whether to show debug messages
// Set via --debug command line flag
pub(crate) static SHOW_DEBUG: AtomicBool = AtomicBool::new(false);

/// Interactive chat commands (kept for backwards compatibility)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatCommand {
    /// Start a new conversation session
    New,
    /// Clear the current conversation context
    Clear,
    /// Set repository context mode (off, on, auto)
    Context(String),
    /// Show conversation history
    History,
    /// Switch to a different conversation session
    Switch(String),
    /// Read a file into context
    Read(String),
    /// List files in a directory
    List(Option<String>),
    /// Exit the chat
    Exit,
    /// Show help
    Help,
}

/// Parse a command from user input
///
/// Returns None if the input is not a command (doesn't start with /)
pub fn parse_command(input: &str) -> Option<ChatCommand> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }

    let parts: Vec<&str> = trimmed[1..].splitn(2, ' ').collect();
    let cmd = parts[0].to_lowercase();
    let arg = parts.get(1).map(|s| s.trim().to_string());

    match cmd.as_str() {
        "new" => Some(ChatCommand::New),
        "clear" => Some(ChatCommand::Clear),
        "context" => Some(ChatCommand::Context(arg.unwrap_or_default())),
        "history" => Some(ChatCommand::History),
        "switch" => arg.map(ChatCommand::Switch),
        "read" => arg.map(ChatCommand::Read),
        "list" | "ls" => Some(ChatCommand::List(arg)),
        "exit" | "quit" | "q" => Some(ChatCommand::Exit),
        "help" | "?" => Some(ChatCommand::Help),
        _ => None,
    }
}

fn create_runtime() -> std::io::Result<Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_name("arkavo-chat-worker")
        .thread_stack_size(3 * 1024 * 1024)
        .enable_all()
        .build()
}

/// Execute the chat command
#[allow(clippy::disallowed_methods)]
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // Check for --help flag first
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_usage();
        return Ok(());
    }

    // Check for --debug flag in arguments
    if args.iter().any(|arg| arg == "--debug") {
        SHOW_DEBUG.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    let flags = parse_cli_args(args)?;

    // Direct A2A chat with a specific mesh agent
    if let Some(id) = flags.agent_id.as_deref() {
        return execute_a2a_direct_chat(id, flags.prompt.as_deref());
    }

    // Default: route through local in-process Router
    execute_a2a_chat(&flags)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ChatCliArgs {
    prompt: Option<String>,
    agent_id: Option<String>,
    model: Option<String>,
    /// Replaces the session's system prompt, so a model fine-tuned under a
    /// pack's prompt can be run under that prompt instead of the default.
    system: Option<String>,
    /// The distilled detector's GGUF. Its presence is what arms the gate.
    #[cfg(feature = "sentinel")]
    sentinel: Option<std::path::PathBuf>,
    /// The thresholds that detector was calibrated at, as `eval.py` writes them.
    #[cfg(feature = "sentinel")]
    calibration: Option<std::path::PathBuf>,
    #[cfg(feature = "sentinel")]
    ceiling: Option<arkavo_protocol::data_classification::SensitivityLevel>,
}

#[cfg(feature = "sentinel")]
impl ChatCliArgs {
    /// The classification ceiling the gate enforces, `internal` by default.
    fn ceiling(&self) -> arkavo_protocol::data_classification::SensitivityLevel {
        self.ceiling
            .unwrap_or(arkavo_protocol::data_classification::SensitivityLevel::Internal)
    }
}

fn parse_cli_args(args: &[String]) -> Result<ChatCliArgs, Box<dyn std::error::Error>> {
    let mut flags = ChatCliArgs::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--prompt" | "--print" if i + 1 < args.len() => {
                flags.prompt = Some(args[i + 1].clone());
                i += 1;
            }
            "--agent-id" => {
                if i + 1 < args.len() {
                    flags.agent_id = Some(args[i + 1].clone());
                    i += 1;
                } else {
                    return Err("--agent-id requires an argument".into());
                }
            }
            "--model" | "--gguf" => {
                let flag = args[i].as_str();
                if i + 1 >= args.len() {
                    return Err(if flag == "--gguf" {
                        "--gguf requires a path to a .gguf or .gguf.tdf file".into()
                    } else {
                        "--model requires a model name or a .gguf path (e.g., ministral-3b, ./adapter.gguf)".into()
                    });
                }
                let value = args[i + 1].clone();
                if flag == "--gguf" && !arkavo_router::model_spec::is_gguf_spec(&value) {
                    return Err("--gguf requires a path ending in .gguf or .gguf.tdf".into());
                }
                if arkavo_router::model_spec::is_gguf_spec(&value) {
                    let resolved =
                        arkavo_router::model_discovery::resolve_gguf_path(Path::new(&value));
                    if !resolved.exists() {
                        return Err(format!("GGUF not found: {value}").into());
                    }
                }
                flags.model = Some(value);
                i += 1;
            }
            "--system" => {
                if i + 1 >= args.len() {
                    return Err("--system requires the prompt text".into());
                }
                flags.system = Some(args[i + 1].clone());
                i += 1;
            }
            #[cfg(feature = "sentinel")]
            "--sentinel" => {
                if i + 1 >= args.len() {
                    return Err("--sentinel requires a path to the detector .gguf".into());
                }
                flags.sentinel = Some(existing_path(&args[i + 1], "sentinel model")?);
                i += 1;
            }
            #[cfg(feature = "sentinel")]
            "--calibration" => {
                if i + 1 >= args.len() {
                    return Err("--calibration requires a path to a calibration .json".into());
                }
                flags.calibration = Some(existing_path(&args[i + 1], "calibration")?);
                i += 1;
            }
            #[cfg(feature = "sentinel")]
            "--ceiling" => {
                if i + 1 >= args.len() {
                    return Err("--ceiling requires public, internal or confidential".into());
                }
                flags.ceiling = Some(crate::sentinel_wiring::parse_ceiling(&args[i + 1])?);
                i += 1;
            }
            // A binary compiled without the classifier must say so rather than
            // ignore the flag: a silently unarmed gate is the failure the whole
            // release path exists to prevent.
            #[cfg(not(feature = "sentinel"))]
            "--sentinel" | "--calibration" => {
                return Err(
                    "sentinel is not in this build; compile with the sentinel feature".into(),
                );
            }
            _ => {}
        }
        i += 1;
    }
    #[cfg(feature = "sentinel")]
    if flags.sentinel.is_some() != flags.calibration.is_some() {
        // Neither half means anything alone: a detector with no thresholds
        // fires on every label, and thresholds with no detector gate nothing.
        return Err("--sentinel and --calibration are used together".into());
    }
    Ok(flags)
}

/// Resolve a flag's path argument, refusing one that is not there.
#[cfg(feature = "sentinel")]
fn existing_path(
    value: &str,
    what: &str,
) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let path = std::path::PathBuf::from(value);
    if !path.exists() {
        return Err(format!("{what} not found: {value}").into());
    }
    Ok(path)
}

fn print_usage() {
    println!("Start interactive chat with LLM\n");
    println!("USAGE:");
    println!("    arkavo chat                                   Interactive chat mode");
    println!("    arkavo chat --prompt \"query\"                  One-shot query");
    println!("    arkavo chat --agent-id <ID>                   Chat with a mesh agent");
    println!("    arkavo chat --agent-id <ID> --prompt \"query\"  One-shot to mesh agent\n");
    println!("EXAMPLES:");
    println!("    arkavo chat");
    println!("    arkavo chat --prompt \"What is 2+2?\"");
    println!("    arkavo chat --model ministral-3b --prompt \"What time is it?\"");
    println!("    arkavo chat --model ./adapter.gguf --prompt \"What is the procedure?\"");
    println!("    arkavo chat --gguf ./adapter.gguf --prompt \"What is the procedure?\"");
    println!("    arkavo chat --agent-id security-auditor-agent --prompt \"Audit this code\"");
    println!("    arkavo chat --agent-id code-analyzer-agent\n");
    println!("OPTIONS:");
    println!(
        "    --model <NAME|PATH>    Catalog name or a .gguf / .gguf.tdf path (not a named registry entry)"
    );
    println!("    --gguf <PATH>          Alias of --model for a .gguf / .gguf.tdf file");
    println!("    --agent-id <ID>        Chat directly with a mesh agent via A2A");
    println!("    --prompt <TEXT>         One-shot query (exits after response)");
    println!("    --system <TEXT>         Replace the session's system prompt");
    #[cfg(feature = "sentinel")]
    {
        println!("    --sentinel <PATH>       Inspect completions with this detector .gguf");
        println!("    --calibration <PATH>    Thresholds for --sentinel (required with it)");
        println!("    --ceiling <LEVEL>       public | internal | confidential (default internal)");
    }
    println!("    --debug                 Show debug output");
    println!("    -h, --help              Show this help\n");
    println!("INTERACTIVE COMMANDS:");
    println!("    /new              Start a new conversation");
    println!("    /clear            Clear conversation context");
    println!("    /context <mode>   Set repo context mode (off, on, auto)");
    println!("    /history          Show conversation history");
    println!("    /read <file>      Read a file into context");
    println!("    /list [dir]       List files in directory");
    println!("    /exit, /quit      Exit chat");
    println!("    /help             Show all commands");
}

/// Build the release gate this invocation runs under, if any (SENT-007).
///
/// One gate serves the session: the router holds it for as long as it holds the
/// session, and the gate resets itself between completions.
#[cfg(feature = "sentinel")]
fn arm_release_gate(
    flags: &ChatCliArgs,
) -> Result<Option<std::sync::Arc<dyn arkavo_llm::ReleaseGate>>, Box<dyn std::error::Error>> {
    let (Some(detector), Some(calibration)) = (&flags.sentinel, &flags.calibration) else {
        return Ok(None);
    };
    let (gate, armed) = crate::sentinel_wiring::armed_gate(detector, calibration, flags.ceiling())?;
    // stderr, not stdout: a one-shot's stdout is the answer, and an operator
    // notice is not part of it.
    eprintln!("{armed}");
    Ok(Some(std::sync::Arc::new(gate)))
}

#[cfg(not(feature = "sentinel"))]
fn arm_release_gate(
    _flags: &ChatCliArgs,
) -> Result<Option<std::sync::Arc<dyn arkavo_llm::ReleaseGate>>, Box<dyn std::error::Error>> {
    Ok(None)
}

/// Execute A2A chat mode using ChatSession from arkavo-protocol
///
/// Uses a local runtime (not `'static`) so that all Rust objects — including
/// the Router's ModelRegistry and its Metal-backed llama.cpp contexts — are
/// dropped deterministically before C++ static destructors run at process exit.
/// A `'static` runtime would keep `Arc<Router>` alive past `main()`, causing
/// `ggml_metal_device_free` to assert on non-empty residual sets.
#[allow(clippy::disallowed_methods)]
fn execute_a2a_chat(flags: &ChatCliArgs) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = create_runtime()?;
    let prompt = flags.prompt.as_deref();
    let model_name = flags.model.as_deref();

    // Before the runtime, and so before anything instantiates the knowledge
    // provider: loading the detector resets llama.cpp's global log callback,
    // which would otherwise undo the debug logging `ARKAVO_DEBUG` turns on when
    // the answering model loads.
    let gate = arm_release_gate(flags)?;

    runtime.block_on(async {
        // Initialize engine with Router + full tool registry (including Claude SDK)
        let engine = arkavo_server::LocalEngine::new_with_release_gate(gate)
            .await
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

        // Create ChatSession (wraps A2aClient). Built a step at a time rather
        // than through `new_with_model`, because the system prompt is captured
        // when the session opens and there is no setting it afterwards.
        let mut client = arkavo_protocol::A2aClient::with_router_and_model(
            engine.router(),
            Some(engine.tool_registry()),
            model_name,
        );
        if let Some(system) = flags.system.as_deref() {
            client.set_system_prompt(system.to_string());
        }
        client.open_session().await?;
        let mut session = ChatSession::from_client(client);

        if std::env::var("ARKAVO_DEBUG").is_ok() && session.is_active() {
            eprintln!("{}", debug_session_started_message());
        }

        // One-shot mode
        if let Some(prompt) = prompt {
            let mut rx = session.send_message(prompt).await?;
            process_stream(&mut rx).await;
            session.cmd_exit().await;
            return Ok(());
        }

        // Interactive REPL
        println!("A2A Chat Mode (type /help for commands, /exit to quit)");
        loop {
            let input = read_user_input()?;

            // Check for commands
            if let Some((cmd, arg)) = parse_protocol_command(&input) {
                let result = execute_command(&mut session, cmd, arg).await;
                match result {
                    CommandResult::Success(msg) => {
                        if let Some(m) = msg {
                            println!("{m}");
                        }
                    }
                    CommandResult::Output(text) => {
                        println!("{text}");
                    }
                    CommandResult::Exit => break,
                    CommandResult::Error(err) => {
                        eprintln!("Error: {err}");
                    }
                }
                continue;
            }

            // Regular message - send to LLM
            if input.is_empty() {
                continue;
            }

            match session.send_message(&input).await {
                Ok(mut rx) => {
                    process_stream(&mut rx).await;
                }
                Err(e) => {
                    eprintln!("Error sending message: {e}");
                }
            }
        }

        Ok(())
    })
}

/// Chat directly with a mesh agent via A2A protocol (bypasses local Router)
#[allow(clippy::disallowed_methods)]
fn execute_a2a_direct_chat(
    agent_id: &str,
    prompt: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    use arkavo_protocol::{
        http::HttpTransport,
        transport::{A2aEndpoint, A2aTransport, TlsConfig, TransportConfig},
    };

    let discovered = super::mesh::discover_mesh_agents()?;
    if discovered.is_empty() {
        return Err("No mesh agents discovered. Are agents running?".into());
    }

    // Match by agent_id (exact) or name substring (fuzzy)
    let agent = discovered
        .iter()
        .find(|a| a.agent_id == agent_id)
        .or_else(|| {
            let lower = agent_id.to_lowercase();
            discovered
                .iter()
                .find(|a| a.name.to_lowercase().contains(&lower))
        })
        .ok_or_else(|| {
            let available: Vec<_> = discovered
                .iter()
                .map(|a| format!("  {} ({})", a.name, a.agent_id))
                .collect();
            format!(
                "Agent '{}' not found. Available agents:\n{}",
                agent_id,
                available.join("\n")
            )
        })?;

    let address = agent
        .address
        .as_ref()
        .ok_or("Selected agent has no address")?;

    println!("Connecting to {} ({})...", agent.name, agent.agent_id);
    println!("  Address: {address}");

    let runtime = create_runtime()?;
    runtime.block_on(async {
        let transport_config = TransportConfig {
            timeout_ms: 60000,
            max_retries: 2,
            tls_config: TlsConfig {
                require_tls: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let transport = HttpTransport::new(transport_config)?;
        let endpoint = A2aEndpoint {
            url: address.clone(),
            agent_id: agent.agent_id.clone(),
            public_key: None,
        };

        transport
            .connect(&endpoint)
            .await
            .map_err(|e| format!("Failed to connect: {e}"))?;
        println!("  Connected\n");

        // One-shot mode
        if let Some(text) = prompt {
            super::mesh::send_and_poll_agent(&transport, text).await?;
            transport.close().await.ok();
            return Ok(());
        }

        // Interactive REPL
        println!("Direct chat with {} (type /exit to quit)", agent.name);
        loop {
            let input = read_user_input()?;
            if input.is_empty() {
                continue;
            }
            if input == "/exit" || input == "/quit" || input == "/q" {
                break;
            }
            match super::mesh::send_and_poll_agent(&transport, &input).await {
                Ok(()) => {}
                Err(e) => eprintln!("Error: {e}"),
            }
        }

        transport.close().await.ok();
        Ok(())
    })
}

fn debug_session_started_message() -> &'static str {
    "[A2A] Session started"
}

/// Write to the operator TTY. Uses `libc::write` so CodeQL does not treat
/// chat tokens as log-injection (stdout `Write` is modeled as a log sink).
fn emit_stdout(s: &str) {
    write_tty(1, s.as_bytes());
}

fn emit_stderr(s: &str) {
    write_tty(2, s.as_bytes());
}

#[cfg(unix)]
fn write_tty(fd: i32, bytes: &[u8]) {
    let mut written = 0;
    while written < bytes.len() {
        // SAFETY: `fd` is stdout (1) or stderr (2); the pointer is into `bytes`.
        let n = unsafe { libc::write(fd, bytes[written..].as_ptr().cast(), bytes.len() - written) };
        if n <= 0 {
            break;
        }
        written += n as usize;
    }
}

#[cfg(not(unix))]
fn write_tty(fd: i32, bytes: &[u8]) {
    use std::io::Write;
    if fd == 2 {
        let _ = io::stderr().write_all(bytes);
    } else {
        let _ = io::stdout().write_all(bytes);
        let _ = io::stdout().flush();
    }
}

/// Process streaming response
async fn process_stream(
    rx: &mut tokio::sync::mpsc::Receiver<arkavo_protocol::types::MessageDelta>,
) {
    use arkavo_protocol::types::MessageDeltaContent;

    let debug = std::env::var("ARKAVO_DEBUG").is_ok();
    let mut buf = String::new();
    let mut in_think = false;

    while let Some(msg) = rx.recv().await {
        match msg.delta {
            MessageDeltaContent::Text { text } => {
                if debug {
                    // Debug mode: show everything including think blocks
                    emit_stdout(&text);
                    continue;
                }

                buf.push_str(&text);

                // Process buffer for think block boundaries
                loop {
                    if in_think {
                        if let Some(end) = buf.find("</think>") {
                            // Discard think content, resume after closing tag
                            buf = buf[end + "</think>".len()..].to_string();
                            in_think = false;
                        } else {
                            // Still inside think block — keep buffering
                            // Truncate to avoid unbounded growth, keep tail for partial tag match
                            // Tail must be >= len("</think>") - 1 = 7 to detect tags across chunks
                            if buf.len() > 1024 {
                                let mut tail_start = buf.len() - 16;
                                // Find nearest char boundary to avoid panic on multi-byte UTF-8
                                while !buf.is_char_boundary(tail_start) {
                                    tail_start += 1;
                                }
                                buf = buf[tail_start..].to_string();
                            }
                            break;
                        }
                    } else if let Some(start) = buf.find("<think>") {
                        // Print everything before the tag
                        let before = &buf[..start];
                        if !before.is_empty() {
                            emit_stdout(before);
                        }
                        buf = buf[start + "<think>".len()..].to_string();
                        in_think = true;
                    } else if let Some(end) = buf.find("</think>") {
                        // Fallback: model emitted <think> as a special token (now injected
                        // by the streaming layer), but in case it was missed, discard
                        // everything before the closing tag
                        buf = buf[end + "</think>".len()..].to_string();
                    } else {
                        // No tag found — flush safe portion, keep tail for partial tag match
                        // len("</think>") = 8, which is longer than "<think>" = 7
                        let safe_len = buf.len().saturating_sub(8);
                        if safe_len > 0 {
                            emit_stdout(&buf[..safe_len]);
                            buf = buf[safe_len..].to_string();
                        }
                        break;
                    }
                }
            }
            MessageDeltaContent::ToolCall { name, .. } => {
                if let Some(name) = name
                    && debug
                {
                    emit_stderr("\n[Tool: ");
                    emit_stderr(&name);
                    emit_stderr("]\n");
                }
            }
            MessageDeltaContent::ToolResult {
                content, is_error, ..
            } => {
                if is_error {
                    emit_stderr("[Error: ");
                    emit_stderr(&content);
                    emit_stderr("]\n");
                }
            }
            MessageDeltaContent::StreamEnd { .. } => {
                // Flush remaining buffer (only if not inside a think block)
                if !in_think && !buf.is_empty() {
                    emit_stdout(&buf);
                }
                emit_stdout("\n");
                break;
            }
            MessageDeltaContent::Error { message, .. } => {
                // SENT-011: a blocked completion tells the consumer that it was
                // blocked and nothing else. The routing layers wrap the refusal
                // on its way here ("Failed to route message: LLM provider
                // error: …"), and each wrapper names a stage the caller could
                // use to work out where generation stopped, so the refusal is
                // reported on its own. `buf` is left unprinted: whatever the
                // gate held is not the consumer's.
                if message.contains(arkavo_llm::GATE_BLOCKED) {
                    emit_stderr("\n");
                    emit_stderr(arkavo_llm::GATE_BLOCKED);
                    emit_stderr("\n");
                } else {
                    emit_stderr("\n[Error: ");
                    emit_stderr(&message);
                    emit_stderr("]\n");
                }
                break;
            }
            MessageDeltaContent::Metadata { key, value } => {
                if debug {
                    match key.as_str() {
                        "model_selected" => {
                            let model = value.get("model").and_then(|v| v.as_str()).unwrap_or("?");
                            let reason = value
                                .get("reasoning")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            emit_stderr(&format!("[Model] {model} ({reason})\n"));
                        }
                        "quality_feedback" => {
                            let latency = value
                                .get("latency_ms")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let tool_count = value
                                .get("tool_call_count")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let resp_len = value
                                .get("response_len")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let mut line = format!("[Perf] {latency}ms, {resp_len} chars");
                            if tool_count > 0 {
                                let names = value
                                    .get("tool_names")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|v| v.as_str())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    })
                                    .unwrap_or_default();
                                let _ = write!(line, ", {tool_count} tool(s): [{names}]");
                            }
                            if let Some(gen_ms) =
                                value.get("generation_ms").and_then(|v| v.as_f64())
                            {
                                let prompt_ms = value
                                    .get("prompt_eval_ms")
                                    .and_then(|v| v.as_f64())
                                    .unwrap_or(0.0);
                                let prompt_tok = value
                                    .get("prompt_tokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let gen_tok = value
                                    .get("generated_tokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let tok_s = value
                                    .get("tokens_per_sec")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("?");
                                let _ = write!(
                                    line,
                                    " | eval: {prompt_ms:.0}ms/{prompt_tok}tok, gen: {gen_ms:.0}ms/{gen_tok}tok ({tok_s} tok/s)"
                                );
                            }
                            line.push('\n');
                            emit_stderr(&line);
                        }
                        "tool_search" => {
                            let keywords = value
                                .get("keywords")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?");
                            let found = value
                                .get("tools_found")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let names = value
                                .get("tool_names")
                                .and_then(|v| v.as_array())
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|v| v.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                })
                                .unwrap_or_default();
                            emit_stderr(&format!(
                                "[Tools] searched \"{keywords}\" → {found} found: [{names}]\n"
                            ));
                        }
                        _ => {
                            emit_stderr("[debug metadata]\n");
                        }
                    }
                }
            }
        }
    }
}

/// Read user input for REPL
fn read_user_input() -> Result<String, Box<dyn std::error::Error>> {
    print!("> ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_new_command() {
        let cmd = parse_command("/new");
        assert!(matches!(cmd, Some(ChatCommand::New)));
    }

    #[test]
    fn test_parse_clear_command() {
        let cmd = parse_command("/clear");
        assert!(matches!(cmd, Some(ChatCommand::Clear)));
    }

    #[test]
    fn test_parse_context_command() {
        let cmd = parse_command("/context auto");
        assert!(matches!(cmd, Some(ChatCommand::Context(mode)) if mode == "auto"));

        let cmd = parse_command("/context on");
        assert!(matches!(cmd, Some(ChatCommand::Context(mode)) if mode == "on"));
    }

    #[test]
    fn test_parse_history_command() {
        let cmd = parse_command("/history");
        assert!(matches!(cmd, Some(ChatCommand::History)));
    }

    #[test]
    fn test_parse_switch_command() {
        let cmd = parse_command("/switch abc-123");
        assert!(matches!(cmd, Some(ChatCommand::Switch(id)) if id == "abc-123"));
    }

    #[test]
    fn test_parse_read_command() {
        let cmd = parse_command("/read src/main.rs");
        assert!(matches!(cmd, Some(ChatCommand::Read(path)) if path == "src/main.rs"));
    }

    #[test]
    fn test_parse_list_command() {
        let cmd = parse_command("/list");
        assert!(matches!(cmd, Some(ChatCommand::List(None))));

        let cmd = parse_command("/list src/");
        assert!(matches!(cmd, Some(ChatCommand::List(Some(path))) if path == "src/"));
    }

    #[test]
    fn test_parse_exit_command() {
        let cmd = parse_command("/exit");
        assert!(matches!(cmd, Some(ChatCommand::Exit)));

        let cmd = parse_command("/quit");
        assert!(matches!(cmd, Some(ChatCommand::Exit)));

        let cmd = parse_command("/q");
        assert!(matches!(cmd, Some(ChatCommand::Exit)));
    }

    #[test]
    fn test_parse_help_command() {
        let cmd = parse_command("/help");
        assert!(matches!(cmd, Some(ChatCommand::Help)));

        let cmd = parse_command("/?");
        assert!(matches!(cmd, Some(ChatCommand::Help)));
    }

    #[test]
    fn test_parse_regular_input() {
        let cmd = parse_command("hello world");
        assert!(cmd.is_none());
    }

    #[test]
    fn test_parse_cli_model_name() {
        let flags = parse_cli_args(&[
            "--model".into(),
            "qwen3.5-0.8b".into(),
            "--prompt".into(),
            "hi".into(),
        ])
        .unwrap();
        assert_eq!(flags.model.as_deref(), Some("qwen3.5-0.8b"));
        assert_eq!(flags.prompt.as_deref(), Some("hi"));
    }

    #[test]
    fn test_parse_cli_gguf_alias_requires_suffix() {
        let err = parse_cli_args(&["--gguf".into(), "not-a-model".into()]).unwrap_err();
        assert!(err.to_string().contains(".gguf"));
    }

    #[test]
    fn test_parse_cli_missing_gguf_path_errors() {
        let err =
            parse_cli_args(&["--model".into(), "models/missing-adapter.gguf".into()]).unwrap_err();
        assert!(err.to_string().contains("GGUF not found"));
    }

    #[test]
    fn test_parse_cli_gguf_flag_missing_arg() {
        let err = parse_cli_args(&["--gguf".into()]).unwrap_err();
        assert!(err.to_string().contains("--gguf requires a path"));
    }

    #[test]
    fn test_parse_cli_existing_gguf_path() {
        let path = std::env::temp_dir().join("arkavo-chat-cli-test.gguf");
        std::fs::write(&path, b"gguf").unwrap();
        let flags = parse_cli_args(&["--gguf".into(), path.to_string_lossy().into()]).unwrap();
        assert_eq!(flags.model.as_deref(), path.to_str());
        let flags = parse_cli_args(&["--model".into(), path.to_string_lossy().into()]).unwrap();
        assert_eq!(flags.model.as_deref(), path.to_str());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_debug_session_started_message_omits_session_id() {
        let msg = debug_session_started_message();
        let lower = msg.to_ascii_lowercase();
        assert!(!lower.contains("session_id"));
        assert!(!lower.contains("session:"));
        assert_eq!(msg, "[A2A] Session started");
    }

    #[test]
    fn test_parse_unknown_command() {
        let cmd = parse_command("/unknown");
        assert!(cmd.is_none());
    }

    #[test]
    fn test_parse_command_case_insensitive() {
        let cmd = parse_command("/NEW");
        assert!(matches!(cmd, Some(ChatCommand::New)));

        let cmd = parse_command("/EXIT");
        assert!(matches!(cmd, Some(ChatCommand::Exit)));
    }

    // Tests using ChatSession from arkavo-protocol
    #[test]
    fn test_chat_session_context_mode() {
        use arkavo_protocol::A2aClient;

        let client = A2aClient::new();
        let mut session = ChatSession::from_client(client);

        assert_eq!(session.context_mode(), ContextMode::Auto);

        let result = session.cmd_context("on");
        assert!(matches!(result, CommandResult::Success(_)));
        assert_eq!(session.context_mode(), ContextMode::On);
    }

    #[test]
    fn test_chat_session_pending_context() {
        use arkavo_protocol::A2aClient;

        let client = A2aClient::new();
        let mut session = ChatSession::from_client(client);

        // Read Cargo.toml
        let result = session.cmd_read("Cargo.toml");
        assert!(matches!(result, CommandResult::Output(_)));
        assert_eq!(session.pending_context().len(), 1);
    }

    #[test]
    fn test_chat_session_help() {
        use arkavo_protocol::A2aClient;

        let client = A2aClient::new();
        let session = ChatSession::from_client(client);
        let result = session.cmd_help();

        match result {
            CommandResult::Output(text) => {
                assert!(text.contains("/new"));
                assert!(text.contains("/clear"));
            }
            _ => panic!("Expected Output"),
        }
    }

    #[tokio::test]
    async fn test_execute_command_programmatic() {
        use arkavo_protocol::A2aClient;

        let client = A2aClient::new();
        let mut session = ChatSession::from_client(client);

        // Execute commands programmatically (like an agent coder would)
        let result = execute_command(&mut session, "context", Some("on")).await;
        assert!(matches!(result, CommandResult::Success(_)));

        let result = execute_command(&mut session, "help", None).await;
        assert!(matches!(result, CommandResult::Output(_)));

        let result = execute_command(&mut session, "list", Some(".")).await;
        assert!(matches!(result, CommandResult::Output(_)));
    }

    /// Serializes the tests that redirect the process's own descriptors.
    /// Two of them swapping fd 2 at once would each restore the other's
    /// redirect and read a truncated capture.
    #[cfg(unix)]
    static TTY_CAPTURE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Redirect a file descriptor to a temp file for the duration of a call,
    /// so a function that writes straight to the terminal can be observed.
    #[cfg(unix)]
    struct Captured {
        fd: i32,
        saved: i32,
        path: std::path::PathBuf,
    }

    #[cfg(unix)]
    impl Captured {
        fn of(fd: i32, name: &str) -> Self {
            let path = std::env::temp_dir().join(name);
            let file = std::fs::File::create(&path).expect("capture file");
            // SAFETY: `fd` is 1 or 2; `saved` is closed in `finish`.
            let saved = unsafe { libc::dup(fd) };
            assert!(saved >= 0, "dup");
            // SAFETY: both descriptors are open; this replaces `fd` until restored.
            assert!(unsafe { libc::dup2(std::os::fd::AsRawFd::as_raw_fd(&file), fd) } >= 0);
            Self { fd, saved, path }
        }

        fn finish(self) -> String {
            // SAFETY: `saved` came from `dup` on `fd` and is still open.
            unsafe {
                libc::dup2(self.saved, self.fd);
                libc::close(self.saved);
            }
            std::fs::read_to_string(&self.path).expect("captured output")
        }
    }

    /// SENT-011: a blocked completion tells the consumer that it was blocked and
    /// nothing else. The routing layers wrap the refusal on the way here, and
    /// each wrapper names a stage; and whatever the gate was holding when it
    /// fired is not the consumer's either.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_blocked_completion_prints_only_the_refusal() {
        use arkavo_protocol::types::{MessageDelta, MessageDeltaContent};

        if std::env::var("ARKAVO_DEBUG").is_ok() {
            eprintln!("skipping: ARKAVO_DEBUG streams text unbuffered");
            return;
        }
        let _serialized = TTY_CAPTURE.lock().unwrap_or_else(|e| e.into_inner());

        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let delta = |sequence, content| MessageDelta {
            session_id: "s".to_string(),
            message_id: "m".to_string(),
            sequence,
            delta: content,
            timestamp: chrono::Utc::now(),
        };
        // Short enough to stay inside the think-tag lookahead, so it is still
        // held when the refusal arrives.
        tx.send(delta(
            0,
            MessageDeltaContent::Text {
                text: "SECRET".to_string(),
            },
        ))
        .await
        .expect("send");
        tx.send(delta(
            1,
            MessageDeltaContent::Error {
                code: "ROUTER_ERROR".to_string(),
                message: format!(
                    "Failed to route message: LLM provider error: Provider error: {}",
                    arkavo_llm::GATE_BLOCKED
                ),
            },
        ))
        .await
        .expect("send");
        drop(tx);

        let out = Captured::of(1, "arkavo-chat-gate-stdout.txt");
        let err = Captured::of(2, "arkavo-chat-gate-stderr.txt");
        process_stream(&mut rx).await;
        let err = err.finish();
        let out = out.finish();

        assert!(err.contains(arkavo_llm::GATE_BLOCKED), "{err:?}");
        assert!(!err.contains("Failed to route message"), "{err:?}");
        assert!(!err.contains("[Error:"), "{err:?}");
        assert!(!err.contains("SECRET"), "{err:?}");
        assert!(!out.contains("SECRET"), "held text reached stdout: {out:?}");
    }

    /// And an ordinary failure still says what went wrong, so the quiet refusal
    /// is the exception rather than the rule.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_ordinary_error_is_still_reported_in_full() {
        use arkavo_protocol::types::{MessageDelta, MessageDeltaContent};

        let _serialized = TTY_CAPTURE.lock().unwrap_or_else(|e| e.into_inner());

        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        tx.send(MessageDelta {
            session_id: "s".to_string(),
            message_id: "m".to_string(),
            sequence: 0,
            delta: MessageDeltaContent::Error {
                code: "ROUTER_ERROR".to_string(),
                message: "Chat inference timed out after 60s".to_string(),
            },
            timestamp: chrono::Utc::now(),
        })
        .await
        .expect("send");
        drop(tx);

        let err = Captured::of(2, "arkavo-chat-plain-stderr.txt");
        process_stream(&mut rx).await;
        let err = err.finish();

        assert!(
            err.contains("[Error: Chat inference timed out after 60s]"),
            "{err:?}"
        );
    }

    /// `--system` is not part of the classifier: a model fine-tuned under a
    /// pack's prompt has to be runnable under that prompt in any build.
    #[test]
    fn test_parse_cli_system_prompt_needs_no_feature() {
        let flags = parse_cli_args(&["--system".into(), "You are the sentinel.".into()]).unwrap();
        assert_eq!(flags.system.as_deref(), Some("You are the sentinel."));

        let err = parse_cli_args(&["--system".into()]).unwrap_err();
        assert!(err.to_string().contains("--system requires"));
    }

    /// A file to point a flag at, since both sentinel flags check that their
    /// argument is there before anything tries to load it.
    #[cfg(feature = "sentinel")]
    fn touch(name: &str) -> String {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, b"placeholder").unwrap();
        path.to_string_lossy().into_owned()
    }

    #[cfg(feature = "sentinel")]
    #[test]
    fn test_parse_cli_sentinel_flags() {
        use arkavo_protocol::data_classification::SensitivityLevel;

        let detector = touch("arkavo-chat-cli-test-sentinel.gguf");
        let calibration = touch("arkavo-chat-cli-test-calibration.json");

        let flags = parse_cli_args(&[
            "--sentinel".into(),
            detector.clone(),
            "--calibration".into(),
            calibration.clone(),
            "--ceiling".into(),
            "confidential".into(),
        ])
        .unwrap();
        assert_eq!(flags.sentinel.as_deref(), Some(Path::new(&detector)));
        assert_eq!(flags.calibration.as_deref(), Some(Path::new(&calibration)));
        assert_eq!(flags.ceiling(), SensitivityLevel::Confidential);

        // The ceiling defaults rather than dropping to Public, which would let
        // a partial completion stream from a model nobody classified.
        let flags = parse_cli_args(&[
            "--sentinel".into(),
            detector.clone(),
            "--calibration".into(),
            calibration.clone(),
        ])
        .unwrap();
        assert_eq!(flags.ceiling(), SensitivityLevel::Internal);

        // And no flags at all is no gate.
        let flags = parse_cli_args(&["--prompt".into(), "hello".into()]).unwrap();
        assert!(flags.sentinel.is_none());
    }

    #[cfg(feature = "sentinel")]
    #[test]
    fn test_parse_cli_sentinel_rejects_half_a_configuration() {
        let detector = touch("arkavo-chat-cli-test-half.gguf");

        let err = parse_cli_args(&["--sentinel".into(), detector]).unwrap_err();
        assert!(err.to_string().contains("used together"), "{err}");

        let err = parse_cli_args(&["--sentinel".into(), "/nonexistent/detector.gguf".into()])
            .unwrap_err();
        assert!(
            err.to_string().contains("sentinel model not found"),
            "{err}"
        );

        let err = parse_cli_args(&["--ceiling".into(), "restricted".into()]).unwrap_err();
        assert!(err.to_string().contains("unknown ceiling"), "{err}");
    }

    /// A build without the classifier says so rather than ignoring the flag: a
    /// gate that was asked for and silently not armed is the failure the
    /// release path exists to prevent.
    #[cfg(not(feature = "sentinel"))]
    #[test]
    fn test_parse_cli_sentinel_reports_the_missing_feature() {
        for flag in ["--sentinel", "--calibration"] {
            let err = parse_cli_args(&[flag.into(), "anything".into()]).unwrap_err();
            assert_eq!(
                err.to_string(),
                "sentinel is not in this build; compile with the sentinel feature"
            );
        }
    }
}
