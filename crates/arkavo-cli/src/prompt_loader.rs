use std::fs;
use std::path::PathBuf;

// Embed all prompt templates at compile time
const CHAT_SYSTEM_PROMPT: &str = include_str!("../../../assets/prompts/chat_system.prompt.md");
const TERMINAL_SYSTEM_PROMPT: &str =
    include_str!("../../../assets/prompts/terminal_system.prompt.md");
const AGENT_SYSTEM_PROMPT: &str = include_str!("../../../assets/prompts/agent_system.prompt.md");
const AGENTS_MD_PROMPT: &str = include_str!("../../../assets/prompts/agents_md.prompt.md");

/// Load a prompt template with override support
///
/// Priority order:
/// 1. User override in .arkavo/prompts/
/// 2. Embedded built-in prompt
/// 3. Fallback to provided default
pub fn load_prompt(prompt_name: &str, default: &str) -> String {
    // Check for user override first
    let user_override_path = get_user_override_path(prompt_name);
    if user_override_path.exists()
        && let Ok(content) = fs::read_to_string(&user_override_path)
    {
        eprintln!("Using custom prompt from: {}", user_override_path.display());
        return process_prompt_template(content);
    }

    // Use embedded built-in prompts
    let builtin_content = match prompt_name {
        "chat_system" => CHAT_SYSTEM_PROMPT,
        "terminal_system" => TERMINAL_SYSTEM_PROMPT,
        "agent_system" => AGENT_SYSTEM_PROMPT,
        "agents_md" => AGENTS_MD_PROMPT,
        _ => return default.to_string(),
    };

    process_prompt_template(builtin_content.to_string())
}

/// Get the path for user override prompts
fn get_user_override_path(prompt_name: &str) -> PathBuf {
    let mut path = PathBuf::from(".arkavo");
    path.push("prompts");
    path.push(format!("{prompt_name}.prompt.md"));
    path
}

/// Process a prompt template file to extract the actual prompt
/// Removes markdown structure and extracts template content
fn process_prompt_template(content: String) -> String {
    // For now, extract content between "## Prompt Template" or "### With MCP Tools" sections
    // This is a simple implementation that looks for the actual prompt content

    let lines: Vec<&str> = content.lines().collect();
    let mut in_template = false;
    let mut template_content = Vec::new();
    let mut found_template = false;

    for line in lines {
        // Start collecting after finding a template marker
        if line.starts_with("## Prompt Template")
            || line.starts_with("### With MCP Tools")
            || line.starts_with("### Without MCP Tools")
        {
            in_template = true;
            found_template = true;
            continue;
        }

        // Stop collecting at next section header
        if in_template && (line.starts_with('#') || line.starts_with("## ")) {
            // Check if we're transitioning between template sections
            if line.starts_with("### ") {
                continue; // Keep collecting for subsections
            }
            break;
        }

        // Collect template content
        if in_template && !line.trim().is_empty() {
            template_content.push(line);
        }
    }

    if found_template && !template_content.is_empty() {
        template_content.join("\n").trim().to_string()
    } else {
        // If no template section found, return the whole content trimmed
        content.trim().to_string()
    }
}

/// Render a prompt template with variables
pub fn render_prompt(template: &str, variables: &[(String, String)]) -> String {
    let mut result = template.to_string();

    for (key, value) in variables {
        // Replace {{key}} with value
        let placeholder = format!("{{{{{key}}}}}");
        result = result.replace(&placeholder, value);

        // Also handle {{#if key}} ... {{/if}} conditionals
        let if_start = format!("{{{{#if {key}}}}}");
        let if_end = "{{/if}}";

        if result.contains(&if_start) {
            if value.is_empty() || value == "false" {
                // Remove the entire conditional block
                while let Some(start_pos) = result.find(&if_start) {
                    if let Some(end_pos) = result[start_pos..].find(if_end) {
                        let end_pos = start_pos + end_pos + if_end.len();
                        result.replace_range(start_pos..end_pos, "");
                    } else {
                        break;
                    }
                }
            } else {
                // Keep the content but remove the conditional markers
                result = result.replace(&if_start, "");
                result = result.replace(if_end, "");
            }
        }
    }

    result
}

/// Load the chat system prompt
pub fn load_chat_system_prompt(mcp_available: bool, available_tools: Option<&str>) -> String {
    // For small models, minimal or no system prompt works best
    if mcp_available {
        let tools_section = if let Some(tools) = available_tools {
            format!("Available tools:\n{tools}")
        } else {
            String::new()
        };

        format!(
            "{tools_section}

Tool Discovery: If you need a capability not listed above, annotate your response with [need: keyword].
Examples: [need: time], [need: github], [need: filesystem]

Context Archives: You may see [ARCHIVED: Summary - ID: uuid] pointers in context.
These are compressed logs. To read them, use the context_restore tool with the ID.
Do NOT guess archived content - always restore it first."
        )
    } else {
        String::new()
    }
}

/// Load the terminal system prompt
pub fn load_terminal_system_prompt(mcp_available: bool, mcp_info: Option<&str>) -> String {
    let template = load_prompt(
        "terminal_system",
        "You are a helpful AI assistant with access to the user's codebase and tools.",
    );

    let mut variables = vec![("mcp_available".to_string(), mcp_available.to_string())];

    if let Some(info) = mcp_info {
        variables.push(("mcp_info".to_string(), info.to_string()));
    } else {
        variables.push(("mcp_info".to_string(), String::new()));
    }

    render_prompt(&template, &variables)
}

/// Load the agent system prompt
pub fn load_agent_system_prompt(
    agent_name: &str,
    agent_purpose: &str,
    mcp_servers: Option<Vec<String>>,
    capabilities: Vec<String>,
) -> String {
    let template = load_prompt(
        "agent_system",
        &format!("You are {agent_name}, a specialized AI agent. Purpose: {agent_purpose}"),
    );

    let mut variables = vec![
        ("agent_name".to_string(), agent_name.to_string()),
        ("agent_purpose".to_string(), agent_purpose.to_string()),
        ("capabilities".to_string(), capabilities.join("\n")),
    ];

    if let Some(servers) = mcp_servers {
        variables.push(("mcp_servers".to_string(), servers.join("\n")));
    } else {
        variables.push(("mcp_servers".to_string(), String::new()));
    }

    render_prompt(&template, &variables)
}

/// Model family for prompt selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFamily {
    Gemma,
    Glm,
    Qwen,
    Mistral,
    DeepSeek,
    Claude,
    Gemini,
    Unknown,
}

/// Model size category for prompt complexity
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSize {
    Tiny,   // <1B params (270M, 500M)
    Small,  // 1-4B params
    Medium, // 4-12B params
    Large,  // 12-30B params
    XLarge, // 30B+ params
}

impl ModelFamily {
    /// Detect model family from model name
    pub fn from_name(name: &str) -> Self {
        let lower = name.to_lowercase();
        if lower.contains("glm") {
            ModelFamily::Glm
        } else if lower.contains("gemma") {
            ModelFamily::Gemma
        } else if lower.contains("qwen") {
            ModelFamily::Qwen
        } else if lower.contains("mistral") || lower.contains("ministral") {
            ModelFamily::Mistral
        } else if lower.contains("deepseek") {
            ModelFamily::DeepSeek
        } else if lower.contains("claude") {
            ModelFamily::Claude
        } else if lower.contains("gemini") {
            ModelFamily::Gemini
        } else {
            ModelFamily::Unknown
        }
    }
}

impl ModelSize {
    /// Detect model size from size hint string
    pub fn from_hint(hint: Option<&str>) -> Self {
        match hint {
            Some("270M" | "500M") => ModelSize::Tiny,
            Some("1B" | "2B" | "3B" | "4B") => ModelSize::Small,
            Some("7B" | "8B" | "9B" | "12B") => ModelSize::Medium,
            Some("14B" | "27B" | "30B") => ModelSize::Large,
            Some("70B" | "72B") => ModelSize::XLarge,
            _ => ModelSize::Medium, // Default assumption
        }
    }
}

/// Load model-specific system prompt for chat
///
/// Different models have different prompting requirements:
/// - GLM: Prefers direct instructions, tends to over-use tools
/// - Gemma: Works well with examples, balanced tool usage
/// - Tiny models: Minimal or no system prompt works best
pub fn load_model_chat_prompt(
    model_name: &str,
    size_hint: Option<&str>,
    tools: Option<&[&str]>,
) -> String {
    let family = ModelFamily::from_name(model_name);
    let size = ModelSize::from_hint(size_hint);

    // Tiny models work best with minimal prompts
    if matches!(size, ModelSize::Tiny) {
        return String::new();
    }

    match family {
        ModelFamily::Glm => load_glm_chat_prompt(size, tools),
        ModelFamily::Gemma => load_gemma_chat_prompt(size, tools),
        ModelFamily::Qwen => load_qwen_chat_prompt(size, tools),
        _ => load_default_chat_prompt(tools),
    }
}

/// GLM-specific chat prompt
/// GLM-4.7-Flash is a MoE (Mixture of Experts) trained as Code Interpreter
/// The coding expert gets triggered by technical/tool prompts and causes fence-wrapping and loops
/// Key: Explicitly route to the CONVERSATION expert, avoid triggering code expert
fn load_glm_chat_prompt(size: ModelSize, _tools: Option<&[&str]>) -> String {
    // GLM MoE routing: Avoid triggering the coding expert
    // Don't mention: code, tools, execute, compute, programming
    // DO mention: conversation, chat, discuss, explain
    match size {
        ModelSize::Tiny => String::new(),
        ModelSize::Small | ModelSize::Medium | ModelSize::Large | ModelSize::XLarge => {
            // Explicitly frame as conversation to route to conversational expert
            "You are having a friendly conversation. Respond naturally in plain text.\n\
             For math: just state the answer.\n\
             For facts: just explain.\n\
             Never use code blocks or special formatting."
                .to_string()
        }
    }
}

/// Gemma-specific chat prompt
/// Gemma models work well with examples and balanced instructions
fn load_gemma_chat_prompt(size: ModelSize, tools: Option<&[&str]>) -> String {
    let tool_section = if let Some(tool_list) = tools {
        if tool_list.is_empty() {
            String::new()
        } else {
            let tools_str = tool_list
                .iter()
                .map(|t| format!("- @{t}"))
                .collect::<Vec<_>>()
                .join("\n");
            format!("\n\nAvailable tools:\n{tools_str}")
        }
    } else {
        String::new()
    };

    match size {
        ModelSize::Tiny => String::new(),
        ModelSize::Small => {
            format!("Be helpful and concise.{tool_section}")
        }
        _ => {
            format!(
                "You are a helpful AI assistant.{tool_section}\n\n\
                 Use tools when needed for: file operations, git status, external APIs.\n\
                 Answer directly for: math, general knowledge, conversation."
            )
        }
    }
}

/// Qwen-specific chat prompt
fn load_qwen_chat_prompt(_size: ModelSize, tools: Option<&[&str]>) -> String {
    // Qwen uses ChatML format and handles tools well
    load_default_chat_prompt(tools)
}

/// Default chat prompt for unknown models
fn load_default_chat_prompt(tools: Option<&[&str]>) -> String {
    if let Some(tool_list) = tools
        && !tool_list.is_empty()
    {
        let tools_str = tool_list
            .iter()
            .map(|t| format!("- {t}"))
            .collect::<Vec<_>>()
            .join("\n");
        return format!(
            "Available tools:\n{tools_str}\n\n\
             Use @tool_name {{\"param\": \"value\"}} to call a tool."
        );
    }
    String::new()
}

/// Initialize the .arkavo/prompts directory if it doesn't exist
pub fn init_prompt_override_dir() -> std::io::Result<()> {
    let mut path = PathBuf::from(".arkavo");
    path.push("prompts");

    if !path.exists() {
        fs::create_dir_all(&path)?;

        // Create a README in the prompts directory
        let readme_path = path.join("README.md");
        let readme_content = r#"# Custom Prompts Directory

Place your custom prompt overrides here to replace the built-in prompts.

## Available Prompts to Override

- `chat_system.prompt.md` - System prompt for chat command
- `terminal_system.prompt.md` - System prompt for terminal UI
- `agent_system.prompt.md` - System prompt for agents
- `agents_md.prompt.md` - Template for generating AGENTS.md files

## How to Override

1. Copy the prompt file from `assets/prompts/` to this directory
2. Modify it as needed
3. The custom version will be used automatically

Example:
```bash
cp ../../assets/prompts/chat_system.prompt.md ./chat_system.prompt.md
# Edit chat_system.prompt.md with your customizations
```

The arkavo CLI will automatically detect and use your custom prompts.
"#;
        fs::write(readme_path, readme_content)?;
    }

    Ok(())
}
