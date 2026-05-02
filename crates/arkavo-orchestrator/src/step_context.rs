//! Inline reasoning trace carried across commands within a step (C5).
//!
//! The original `do_step` re-created a fresh `user` message for every
//! command, causing the model to lose context about what it had just done.
//! This helper accumulates a compact history of prior command outcomes
//! and injects them into subsequent prompts, restoring ReAct-like
//! coherence without blowing the token budget.

use crate::text_utils::truncate_ellipsis;
use arkavo_llm::{Message as LlmMessage, ProviderResponse};

/// Maximum characters of accumulated trace to keep in the prompt.
const MAX_TRACE_CHARS: usize = 4000;

/// Maximum number of prior command outcomes to remember.
const MAX_TRACE_ENTRIES: usize = 8;

/// A compact summary of one executed command, suitable for injecting back
/// into the next LLM turn.
#[derive(Debug, Clone)]
pub struct TraceEntry {
    pub command: String,
    pub summary: String,
    pub tool_calls: usize,
}

/// Accumulator for the rolling reasoning trace within a single step.
#[derive(Debug, Clone, Default)]
pub struct StepTrace {
    entries: Vec<TraceEntry>,
}

impl StepTrace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the outcome of a command, summarizing it for future prompts.
    pub fn record(&mut self, command: &str, response: &ProviderResponse) {
        let summary = summarize_response(response);
        self.entries.push(TraceEntry {
            command: truncate_ellipsis(command, 200),
            summary,
            tool_calls: response.tool_calls.len(),
        });
        // Cap memory use: keep the most recent entries.
        if self.entries.len() > MAX_TRACE_ENTRIES {
            let drop_n = self.entries.len() - MAX_TRACE_ENTRIES;
            self.entries.drain(0..drop_n);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Render the trace as a prompt-friendly context block, bounded by
    /// `MAX_TRACE_CHARS`.
    pub fn render(&self) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let mut out = String::from(
            "Previous commands in this step and their outcomes (use this context to avoid redundant work):\n",
        );
        for (i, e) in self.entries.iter().enumerate() {
            let line = format!(
                "{}. `{}` \u{2192} {} tool call(s); {}\n",
                i + 1,
                e.command,
                e.tool_calls,
                e.summary
            );
            if out.len() + line.len() > MAX_TRACE_CHARS {
                out.push_str("\u{2026} (older entries elided)\n");
                break;
            }
            out.push_str(&line);
        }
        Some(out)
    }

    /// Render the full prompt content that `build_messages` would send,
    /// suitable for fallback token accounting when the provider does not
    /// report `InferenceTiming`. Keeps token estimates honest by including
    /// the prepended trace, not just the raw `task_prompt`.
    pub fn rendered_prompt(&self, task_prompt: &str) -> String {
        match self.render() {
            Some(trace) => format!("{trace}\n---\n{task_prompt}"),
            None => task_prompt.to_string(),
        }
    }

    /// Build the message vector for the next command, prepending the
    /// rolling trace as a system-style preamble.
    pub fn build_messages(&self, task_prompt: String) -> Vec<LlmMessage> {
        vec![LlmMessage::user(self.rendered_prompt(&task_prompt))]
    }
}

/// Build a short (1-2 line) textual summary of what the provider returned.
fn summarize_response(response: &ProviderResponse) -> String {
    let mut parts = Vec::new();
    let content_trimmed = response.content.trim();
    if !content_trimmed.is_empty() {
        parts.push(truncate_ellipsis(content_trimmed, 240));
    }
    if !response.tool_calls.is_empty() {
        let tool_names: Vec<&str> = response
            .tool_calls
            .iter()
            .map(|tc| tc.tool_name.as_str())
            .collect();
        parts.push(format!("tools=[{}]", tool_names.join(",")));
    }
    if let Some(reason) = &response.finish_reason {
        parts.push(format!("finish={reason}"));
    }
    if parts.is_empty() {
        "(no content)".to_string()
    } else {
        parts.join(" | ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(content: &str) -> ProviderResponse {
        ProviderResponse {
            content: content.to_string(),
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: None,
            inference_timing: None,
            quality_gate_retries: 0,
        }
    }

    #[test]
    fn test_empty_trace_renders_none() {
        let t = StepTrace::new();
        assert!(t.is_empty());
        assert!(t.render().is_none());
    }

    #[test]
    fn test_single_entry_rendered() {
        let mut t = StepTrace::new();
        t.record("cat foo.rs", &resp("printed 42 lines"));
        let rendered = t.render().unwrap();
        assert!(rendered.contains("cat foo.rs"));
        assert!(rendered.contains("printed 42 lines"));
    }

    #[test]
    fn test_build_messages_without_trace() {
        let t = StepTrace::new();
        let msgs = t.build_messages("do the thing".to_string());
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn test_build_messages_with_trace() {
        let mut t = StepTrace::new();
        t.record("ls -la", &resp("listed files"));
        let msgs = t.build_messages("now do X".to_string());
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].content.contains("ls -la"));
        assert!(msgs[0].content.contains("listed files"));
        assert!(msgs[0].content.contains("now do X"));
    }

    #[test]
    fn test_cap_entries() {
        let mut t = StepTrace::new();
        for i in 0..20 {
            t.record(&format!("cmd{i}"), &resp(&format!("ok{i}")));
        }
        assert!(t.entries.len() <= MAX_TRACE_ENTRIES);
        // Oldest should have been dropped.
        assert!(!t.entries.iter().any(|e| e.command == "cmd0"));
    }

    #[test]
    fn test_summary_with_no_content() {
        let s = summarize_response(&resp(""));
        assert_eq!(s, "(no content)");
    }

    #[test]
    fn test_rendered_prompt_matches_message_content() {
        // Regression: token accounting must use rendered_prompt to capture the
        // expanded trace + task content actually sent to the provider.
        let mut t = StepTrace::new();
        t.record("ls", &resp("listed"));
        let task = "next thing to do";
        let rendered = t.rendered_prompt(task);
        let msgs = t.build_messages(task.to_string());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, rendered);
        assert!(rendered.contains("ls"));
        assert!(rendered.contains(task));
    }

    #[test]
    fn test_rendered_prompt_empty_trace_is_just_task() {
        let t = StepTrace::new();
        assert_eq!(t.rendered_prompt("only the task"), "only the task");
    }
}
