//! Prompt formatting utilities for TØR-G synthesis
//!
//! Provides system prompts and chat template formatting for LLM-based
//! policy graph generation.

use arkavo_llama_cpp::ModelFormat;

/// System prompt for TØR-G generation
///
/// Format uses single-character tokens:
/// - `<` followed by number for input declaration
/// - `>` followed by number for output declaration
/// - `[` `]` for node bounds
/// - `|` `!` `^` for OR, NOR, XOR operators
pub const TORG_SYSTEM_PROMPT: &str = r#"Generate TØR-G boolean circuits.

Syntax: <id declares input, [id op a b] defines node, >id declares output.
Operators: | (OR), ! (NOR), ^ (XOR)

Examples:
"A OR B" -> <0 <1 [2 | 0 1] >2
"X XOR Y" -> <0 <1 [2 ^ 0 1] >2
"NOT A" -> <0 [1 ! 0 0] >1

Output ONLY the tokens."#;

/// Format a policy description using the appropriate chat template for the model
///
/// # Arguments
///
/// * `policy` - Natural language description of the policy to generate
/// * `format` - The model format (determines chat template)
///
/// # Returns
///
/// A formatted prompt string with system instructions and the policy request.
#[must_use]
pub fn format_prompt(policy: &str, format: ModelFormat) -> String {
    match format {
        ModelFormat::Qwen3 => {
            // ChatML format for Qwen models
            format!(
                "<|im_start|>system\n{TORG_SYSTEM_PROMPT}<|im_end|>\n\
                 <|im_start|>user\n{policy}<|im_end|>\n\
                 <|im_start|>assistant\n"
            )
        }
        ModelFormat::MistralV3 => {
            // Mistral V3 format for Ministral models
            format!(
                "[SYSTEM_PROMPT]{TORG_SYSTEM_PROMPT}[/SYSTEM_PROMPT]\
                 [INST] {policy} [/INST]"
            )
        }
        ModelFormat::Gemma3 | ModelFormat::Gemma4 => {
            // Gemma 3/4 format (same turn markers)
            format!(
                "<start_of_turn>user\n{TORG_SYSTEM_PROMPT}\n\n{policy}<end_of_turn>\n\
                 <start_of_turn>model\n"
            )
        }
        ModelFormat::GLM4 => {
            // GLM-4 format (ChatML variant)
            format!("[gMASK]<sop><|system|>\n{TORG_SYSTEM_PROMPT}<|user|>\n{policy}<|assistant|>\n")
        }
        ModelFormat::Completion => policy.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_prompt_contains_examples() {
        assert!(TORG_SYSTEM_PROMPT.contains("A OR B"));
        assert!(TORG_SYSTEM_PROMPT.contains("X XOR Y"));
        assert!(TORG_SYSTEM_PROMPT.contains("NOT A"));
    }

    #[test]
    fn test_format_prompt_qwen3() {
        let prompt = format_prompt("admin OR owner", ModelFormat::Qwen3);
        assert!(prompt.contains("<|im_start|>system"));
        assert!(prompt.contains("<|im_end|>"));
        assert!(prompt.contains("admin OR owner"));
        assert!(prompt.contains(TORG_SYSTEM_PROMPT));
    }

    #[test]
    fn test_format_prompt_mistral() {
        let prompt = format_prompt("is_public", ModelFormat::MistralV3);
        assert!(prompt.contains("[SYSTEM_PROMPT]"));
        assert!(prompt.contains("[INST]"));
        assert!(prompt.contains("is_public"));
    }

    #[test]
    fn test_format_prompt_gemma3() {
        let prompt = format_prompt("allow access", ModelFormat::Gemma3);
        assert!(prompt.contains("<start_of_turn>user"));
        assert!(prompt.contains("<start_of_turn>model"));
        assert!(prompt.contains("allow access"));
    }

    #[test]
    fn test_format_prompt_glm4() {
        let prompt = format_prompt("admin check", ModelFormat::GLM4);
        assert!(prompt.contains("[gMASK]<sop>"));
        assert!(prompt.contains("<|system|>"));
        assert!(prompt.contains("<|user|>"));
        assert!(prompt.contains("<|assistant|>"));
        assert!(prompt.contains("admin check"));
        assert!(prompt.contains(TORG_SYSTEM_PROMPT));
    }
}
