use serde::{Deserialize, Serialize};

/// Configuration for local LLM inference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    /// Maximum tokens to generate (default: 60)
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,

    /// Maximum prompt tokens allowed (default: 4096)
    #[serde(default = "default_max_prompt_tokens")]
    pub max_prompt_tokens: usize,

    /// Temperature for sampling (default: 0.8)
    #[serde(default = "default_temperature")]
    pub temperature: f64,

    /// Top-p for nucleus sampling (default: 0.95)
    #[serde(default = "default_top_p")]
    pub top_p: Option<f64>,

    /// Random seed for reproducibility (default: 42)
    #[serde(default = "default_seed")]
    pub seed: u64,

    /// Device preference: "auto", "cpu", "metal" (default: "auto")
    #[serde(default = "default_device")]
    pub device: String,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            max_tokens: default_max_tokens(),
            max_prompt_tokens: default_max_prompt_tokens(),
            temperature: default_temperature(),
            top_p: default_top_p(),
            seed: default_seed(),
            device: default_device(),
        }
    }
}

fn default_max_tokens() -> usize {
    60
}

fn default_max_prompt_tokens() -> usize {
    4096
}

fn default_temperature() -> f64 {
    0.8
}

fn default_top_p() -> Option<f64> {
    Some(0.95)
}

fn default_seed() -> u64 {
    42
}

fn default_device() -> String {
    "auto".to_string()
}
