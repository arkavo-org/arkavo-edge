use std::collections::VecDeque;
use std::fmt::Write;

/// Sliding window memory for recent tool calls and responses
#[derive(Debug, Clone, Default)]
pub struct ToolMemory {
    entries: VecDeque<ToolMemoryEntry>,
    max_entries: usize,
}

#[derive(Debug, Clone)]
pub struct ToolMemoryEntry {
    pub tool_name: String,
    pub args_summary: String,
    pub result_summary: String,
    pub timestamp: std::time::Instant,
}

impl ToolMemory {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(max_entries),
            max_entries,
        }
    }

    pub fn add(&mut self, tool_name: String, args: &serde_json::Value, result: &str) {
        let args_summary = serde_json::to_string(args)
            .unwrap_or_default()
            .chars()
            .take(100)
            .collect();
        let result_summary: String = result.chars().take(200).collect();

        if self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }
        self.entries.push_back(ToolMemoryEntry {
            tool_name,
            args_summary,
            result_summary,
            timestamp: std::time::Instant::now(),
        });
    }

    pub fn format_for_prompt(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }
        let mut output = String::from("\n\n## Recent Actions\n");
        for (i, entry) in self.entries.iter().enumerate() {
            let _ = writeln!(
                output,
                "{}. {} {} → {}",
                i + 1,
                entry.tool_name,
                entry.args_summary,
                entry.result_summary
            );
        }
        output
    }
}
