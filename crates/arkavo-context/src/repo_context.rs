//! Repository context management
//!
//! Determines when to attach repository context to prompts
//! and handles context compression for token efficiency.

use std::sync::atomic::{AtomicBool, Ordering};

/// Debug flag for repo context
static SHOW_DEBUG: AtomicBool = AtomicBool::new(false);

/// Set debug mode
pub fn set_debug(enabled: bool) {
    SHOW_DEBUG.store(enabled, Ordering::SeqCst);
}

/// Check if debug mode is enabled
pub fn is_debug() -> bool {
    SHOW_DEBUG.load(Ordering::Relaxed)
}

/// Repository context mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepoContextMode {
    /// Never attach repository context
    Off,
    /// Always attach repository context
    On,
    /// Automatically detect when context is needed
    #[default]
    Auto,
}

impl std::str::FromStr for RepoContextMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "off" | "false" | "no" | "0" => Ok(Self::Off),
            "on" | "true" | "yes" | "1" => Ok(Self::On),
            "auto" | "automatic" => Ok(Self::Auto),
            _ => Err(format!("Invalid repo context mode: {}", s)),
        }
    }
}

impl std::fmt::Display for RepoContextMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => write!(f, "off"),
            Self::On => write!(f, "on"),
            Self::Auto => write!(f, "auto"),
        }
    }
}

/// Determines if repository context should be attached based on the prompt
pub fn should_attach_repo_context(prompt: &str, mode: RepoContextMode) -> bool {
    match mode {
        RepoContextMode::Off => false,
        RepoContextMode::On => true,
        RepoContextMode::Auto => should_attach_auto(prompt),
    }
}

/// Auto-detect whether context should be attached
fn should_attach_auto(prompt: &str) -> bool {
    let p = prompt.trim();

    // Short math/calculation pattern - no context needed
    if p.len() < 32 {
        let is_math = p.chars().all(|c| "0123456789+-*/().=? \t\n".contains(c));
        if is_math {
            if is_debug() {
                eprintln!("[DEBUG] RepoCtx: skipped (short math expression)");
            }
            return false;
        }
    }

    // Greeting patterns - no context needed
    let greetings = [
        "hi",
        "hello",
        "hey",
        "good morning",
        "good afternoon",
        "good evening",
    ];
    let lower = p.to_lowercase();
    if p.len() < 20
        && greetings
            .iter()
            .any(|g| lower == *g || lower == format!("{g}!"))
    {
        if is_debug() {
            eprintln!("[DEBUG] RepoCtx: skipped (greeting)");
        }
        return false;
    }

    // Keywords that indicate code/doc related queries - context needed
    let code_keywords = [
        "readme",
        "build",
        "compile",
        "cargo",
        "rust",
        "error",
        "panic",
        ".rs",
        ".gd",
        ".swift",
        ".toml",
        "arkavo",
        "how do i",
        "how to",
        "api",
        "design",
        "function",
        "class",
        "struct",
        "impl",
        "trait",
        "module",
        "import",
        "test",
        "debug",
        "fix",
        "implement",
        "code",
        "file",
        "directory",
        "git",
        "branch",
        "commit",
        "repository",
        "project",
        "crate",
        "deploy",
        "refactor",
    ];

    if code_keywords.iter().any(|k| lower.contains(k)) {
        if is_debug() {
            eprintln!("[DEBUG] RepoCtx: will inject (code/doc keyword detected)");
        }
        return true;
    }

    // Default: no context for unrecognized patterns
    if is_debug() {
        eprintln!("[DEBUG] RepoCtx: skipped (no code keywords found)");
    }
    false
}

/// Compress repository context to fit within token budget
pub fn compress_repo_context(full_content: &str, max_tokens: usize) -> String {
    let header = "[PROJECT CONTEXT DIGEST — KEEP ANSWER CONCISE]\n";

    // Estimate ~4 chars per token (rough approximation)
    let max_chars = max_tokens * 4;

    if full_content.len() <= max_chars {
        format!("{header}{full_content}")
    } else {
        // Take key sections: title, overview, and first few command examples
        let lines: Vec<&str> = full_content.lines().collect();
        let mut digest = String::from(header);
        let mut token_budget = max_tokens.saturating_sub(20); // Reserve some for header

        for line in lines {
            // Prioritize headers and overview sections
            if line.starts_with("# ")
                || line.starts_with("## Project Overview")
                || line.starts_with("## Key Command")
                || line.contains("arkavo")
            {
                digest.push_str(line);
                digest.push('\n');
                token_budget = token_budget.saturating_sub(line.len() / 4);
                if token_budget < 50 {
                    break;
                }
            }
        }

        digest.push_str("\n(Use 'show docs' for more details)\n");
        digest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_attach_context_mode_off() {
        assert!(!should_attach_repo_context(
            "any prompt",
            RepoContextMode::Off
        ));
    }

    #[test]
    fn test_should_attach_context_mode_on() {
        assert!(should_attach_repo_context("hello", RepoContextMode::On));
    }

    #[test]
    fn test_should_attach_context_auto_code_keywords() {
        assert!(should_attach_repo_context(
            "show me the readme",
            RepoContextMode::Auto
        ));
        assert!(should_attach_repo_context(
            "cargo build error",
            RepoContextMode::Auto
        ));
        assert!(should_attach_repo_context(
            "implement struct Foo",
            RepoContextMode::Auto
        ));
    }

    #[test]
    fn test_should_attach_context_auto_greetings() {
        assert!(!should_attach_repo_context("hello", RepoContextMode::Auto));
        assert!(!should_attach_repo_context("hi", RepoContextMode::Auto));
    }

    #[test]
    fn test_should_attach_context_auto_math() {
        assert!(!should_attach_repo_context("2+2", RepoContextMode::Auto));
    }

    #[test]
    fn test_compress_repo_context() {
        let full_context = "# Project\n".repeat(1000);
        let max_tokens = 100;
        let compressed = compress_repo_context(&full_context, max_tokens);
        assert!(compressed.len() < full_context.len());
        assert!(compressed.contains("# Project"));
    }

    #[test]
    fn test_compress_repo_context_small_input() {
        let small_context = "# Project\nSmall content.";
        let compressed = compress_repo_context(small_context, 1000);
        assert!(compressed.contains(small_context));
    }

    #[test]
    fn test_repo_context_mode_from_str() {
        assert_eq!(
            "off".parse::<RepoContextMode>().unwrap(),
            RepoContextMode::Off
        );
        assert_eq!(
            "on".parse::<RepoContextMode>().unwrap(),
            RepoContextMode::On
        );
        assert_eq!(
            "auto".parse::<RepoContextMode>().unwrap(),
            RepoContextMode::Auto
        );
    }

    #[test]
    fn test_repo_context_mode_display() {
        assert_eq!(RepoContextMode::Off.to_string(), "off");
        assert_eq!(RepoContextMode::On.to_string(), "on");
        assert_eq!(RepoContextMode::Auto.to_string(), "auto");
    }

    #[test]
    fn test_debug_flag() {
        set_debug(true);
        assert!(is_debug());
        set_debug(false);
        assert!(!is_debug());
    }

    /// CTX-015: Verify compressed output has the expected header and footer
    #[test]
    fn test_compress_output_format() {
        // Build content large enough to trigger compression (>1000 tokens ~ >4000 chars)
        let large_content = "Some code line\n".repeat(500);
        let compressed = compress_repo_context(&large_content, 200);
        assert!(
            compressed.contains("[PROJECT CONTEXT DIGEST"),
            "Compressed output must contain the digest header"
        );
        assert!(
            compressed.contains("(Use 'show docs' for more details)"),
            "Compressed output must contain the footer hint"
        );
    }

    /// CTX-015: Compression preserves section headers from input
    #[test]
    fn test_compress_preserves_headers() {
        let input = "# Authentication\ncode here\n# Database\nmore code\n";
        // Use large repetition to force compression path
        let large_input = format!("{}{}", input, "filler line\n".repeat(500));
        let compressed = compress_repo_context(&large_input, 200);
        assert!(
            compressed.contains("# Authentication"),
            "Compressed output must preserve '# Authentication' header"
        );
        assert!(
            compressed.contains("# Database"),
            "Compressed output must preserve '# Database' header"
        );
    }

    /// CTX-014: Auto mode detects all expected code-related keywords
    #[test]
    fn test_auto_detects_code_keywords_comprehensive() {
        let keywords = [
            "function", "class", "module", "import", "error", "compile", "build", "test", "deploy",
            "refactor",
        ];
        for keyword in &keywords {
            let prompt = format!("Tell me about {} handling", keyword);
            assert!(
                should_attach_repo_context(&prompt, RepoContextMode::Auto),
                "Auto mode should attach context for keyword '{}'",
                keyword
            );
        }
    }
}
