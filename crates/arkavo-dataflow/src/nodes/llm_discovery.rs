use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Information about available LLM capabilities for agent discovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCapabilityInfo {
    pub providers: Vec<ProviderInfo>,
    pub example_patterns: Vec<PatternExample>,
    pub task_types: Vec<TaskTypeInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub name: String,
    pub description: String,
    pub example_url: String,
    pub suitable_for: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternExample {
    pub name: String,
    pub description: String,
    pub use_case: String,
    pub blueprint_snippet: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTypeInfo {
    pub task_type: String,
    pub description: String,
    pub recommended_models: Vec<String>,
    pub example_prompt: String,
}

/// Discovers available LLM providers by checking connectivity
pub async fn discover_ollama_providers() -> Result<Vec<(String, String)>> {
    let mut discovered = Vec::new();
    
    // Check local Ollama
    if check_ollama_endpoint("http://localhost:11434").await.is_ok() {
        discovered.push(("local-ollama".to_string(), "http://localhost:11434".to_string()));
    }
    
    // Check common local network addresses
    let common_ports = vec!["11434", "8080", "8000"];
    let subnet_prefixes = vec!["192.168.1", "192.168.0", "10.0.0", "172.16.0"];
    
    for prefix in &subnet_prefixes {
        for i in 1..255 {
            for port in &common_ports {
                let url = format!("http://{}.{}:{}", prefix, i, port);
                if check_ollama_endpoint(&url).await.is_ok() {
                    discovered.push((format!("remote-{}-{}", prefix, i), url));
                }
            }
        }
    }
    
    // Check environment variable
    if let Ok(remote_url) = std::env::var("REMOTE_OLLAMA_URL") {
        if check_ollama_endpoint(&remote_url).await.is_ok() {
            discovered.push(("remote-env".to_string(), remote_url));
        }
    }
    
    Ok(discovered)
}

async fn check_ollama_endpoint(url: &str) -> Result<()> {
    // This would make an HTTP request to check if Ollama is running
    // For now, we'll simulate the check
    if url.contains("localhost") {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Endpoint not reachable"))
    }
}

/// Get information about LLM capabilities for agent self-discovery
pub fn get_llm_capability_info() -> LlmCapabilityInfo {
    LlmCapabilityInfo {
        providers: vec![
            ProviderInfo {
                name: "local-ollama".to_string(),
                description: "Local Ollama instance for development and testing".to_string(),
                example_url: "http://localhost:11434".to_string(),
                suitable_for: vec!["development", "testing", "quick-tasks"].into_iter().map(String::from).collect(),
            },
            ProviderInfo {
                name: "remote-ollama".to_string(),
                description: "Remote Ollama instance for specialized models".to_string(),
                example_url: "http://10.0.0.101:11434".to_string(),
                suitable_for: vec!["production", "specialized-models", "high-performance"].into_iter().map(String::from).collect(),
            },
        ],
        example_patterns: vec![
            PatternExample {
                name: "summarization".to_string(),
                description: "Summarize incoming data using LLM".to_string(),
                use_case: "Condensing long text into key points".to_string(),
                blueprint_snippet: json!({
                    "type": "llm_transform",
                    "provider": "local-ollama",
                    "model": "llama3.2:latest",
                    "prompt": "Summarize this in 2-3 sentences: {{input}}",
                    "temperature": 0.5
                }),
            },
            PatternExample {
                name: "code_review".to_string(),
                description: "Review code for quality and issues".to_string(),
                use_case: "Automated code analysis and feedback".to_string(),
                blueprint_snippet: json!({
                    "type": "llm_transform",
                    "provider": "local-ollama",
                    "model": "devstral:latest",
                    "task_type": "code_review",
                    "prompt": "Review this code for quality, bugs, and security issues: {{input}}",
                    "temperature": 0.2
                }),
            },
            PatternExample {
                name: "content_routing".to_string(),
                description: "Route content based on LLM classification".to_string(),
                use_case: "Dynamic pipeline routing based on content type".to_string(),
                blueprint_snippet: json!({
                    "type": "llm_transform",
                    "provider": "local-ollama",
                    "model": "qwen3:0.6b",
                    "prompt": "Classify this as: technical, business, or general. Reply with one word only: {{input}}",
                    "temperature": 0.1
                }),
            },
        ],
        task_types: vec![
            TaskTypeInfo {
                task_type: "code_review".to_string(),
                description: "Review code for quality, bugs, and best practices".to_string(),
                recommended_models: vec!["devstral:latest", "codellama:latest"].into_iter().map(String::from).collect(),
                example_prompt: "Review the following code and provide feedback on quality, potential bugs, and improvements".to_string(),
            },
            TaskTypeInfo {
                task_type: "summarization".to_string(),
                description: "Create concise summaries of longer content".to_string(),
                recommended_models: vec!["llama3.2:latest", "mistral:latest"].into_iter().map(String::from).collect(),
                example_prompt: "Summarize the following content in 2-3 sentences".to_string(),
            },
            TaskTypeInfo {
                task_type: "translation".to_string(),
                description: "Translate text between languages".to_string(),
                recommended_models: vec!["qwen3:latest", "llama3.2:latest"].into_iter().map(String::from).collect(),
                example_prompt: "Translate the following text to Spanish".to_string(),
            },
            TaskTypeInfo {
                task_type: "classification".to_string(),
                description: "Categorize content into predefined classes".to_string(),
                recommended_models: vec!["qwen3:0.6b", "phi3:mini"].into_iter().map(String::from).collect(),
                example_prompt: "Classify this content into one of these categories: technical, business, general".to_string(),
            },
        ],
    }
}

/// Generate a blueprint node configuration based on requirements
pub fn suggest_llm_node_config(
    task_description: &str,
    available_providers: &[(String, String)],
) -> Result<serde_json::Value> {
    // Analyze task description to determine best configuration
    let task_lower = task_description.to_lowercase();
    
    let (task_type, model, temperature) = if task_lower.contains("code") || task_lower.contains("review") {
        ("code_review", "devstral:latest", 0.2)
    } else if task_lower.contains("summar") {
        ("summarization", "llama3.2:latest", 0.5)
    } else if task_lower.contains("translat") {
        ("translation", "qwen3:latest", 0.3)
    } else if task_lower.contains("classif") || task_lower.contains("route") {
        ("classification", "qwen3:0.6b", 0.1)
    } else {
        ("general", "llama3.2:latest", 0.7)
    };
    
    // Select provider based on model availability
    let provider = if available_providers.len() > 1 && model == "devstral:latest" {
        "remote-ollama" // Prefer remote for specialized models
    } else {
        "local-ollama"
    };
    
    Ok(json!({
        "type": "llm_transform",
        "provider": provider,
        "model": model,
        "task_type": task_type,
        "temperature": temperature,
        "prompt": format!("{}: {{{{input}}}}", task_description),
        "timeout_secs": 30
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_info() {
        let info = get_llm_capability_info();
        assert!(!info.providers.is_empty());
        assert!(!info.example_patterns.is_empty());
        assert!(!info.task_types.is_empty());
    }

    #[test]
    fn test_suggest_config() {
        let providers = vec![
            ("local-ollama".to_string(), "http://localhost:11434".to_string()),
        ];
        
        let config = suggest_llm_node_config("Review this code for bugs", &providers).unwrap();
        assert_eq!(config["task_type"], "code_review");
        assert_eq!(config["model"], "devstral:latest");
    }
}