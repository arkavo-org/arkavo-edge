use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Includes cached input.
    pub input_tokens: u32,
    pub cached_input_tokens: u32,
    /// Part of `input_tokens` that was written into the prompt cache. Priced
    /// at the cache-write rate as a total for those tokens, not as a surcharge
    /// on the input rate, so each token is charged exactly once. Optional in
    /// Codex's own `Usage`, and absence must not be read as a discount.
    #[serde(default)]
    pub cache_write_input_tokens: u32,
    /// Includes reasoning output.
    pub output_tokens: u32,
    #[serde(default)]
    pub reasoning_output_tokens: u32,
}

impl Usage {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.cached_input_tokens <= self.input_tokens,
            "Invalid cached usage"
        );
        ensure!(
            self.cache_write_input_tokens <= self.input_tokens,
            "Invalid cache write usage"
        );
        ensure!(
            self.reasoning_output_tokens <= self.output_tokens,
            "Invalid reasoning usage"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub kind: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Completed,
    #[default]
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunOutcome {
    pub thread_id: Option<String>,
    pub status: RunStatus,
    pub message: String,
    pub changes: Vec<FileChange>,
    pub usage: Option<Usage>,
    /// API-price estimate, including when Codex uses a subscription.
    pub estimated_cost_usd: Option<f64>,
    /// Missing usage is never silently treated as zero spending.
    pub accounting_incomplete: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub(crate) struct EventReader {
    pub(crate) outcome: RunOutcome,
    terminal: bool,
}

impl EventReader {
    pub(crate) fn accept(&mut self, line: &[u8]) -> Result<()> {
        let event: Value = serde_json::from_slice(line)?;
        let kind = event["type"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Event has no type"))?;
        ensure!(!self.terminal, "Event received after terminal event");
        match kind {
            "thread.started" => {
                let id = event["thread_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing thread ID"))?;
                uuid::Uuid::parse_str(id)?;
                ensure!(
                    self.outcome
                        .thread_id
                        .as_deref()
                        .is_none_or(|old| old == id),
                    "Thread ID changed"
                );
                self.outcome.thread_id = Some(id.into());
            }
            "item.completed" => {
                let item = &event["item"];
                match item["type"].as_str() {
                    Some("agent_message") => {
                        self.outcome.message = item["text"].as_str().unwrap_or_default().into();
                    }
                    Some("file_change") if item["status"] == "completed" => {
                        let changes: Vec<FileChange> =
                            serde_json::from_value(item["changes"].clone())?;
                        self.outcome.changes.extend(changes);
                    }
                    _ => {}
                }
            }
            "turn.completed" => {
                let usage: Usage = serde_json::from_value(event["usage"].clone())?;
                usage.validate()?;
                self.outcome.usage = Some(usage);
                self.outcome.status = RunStatus::Completed;
                self.terminal = true;
            }
            "turn.failed" => {
                self.outcome.error = Some("Codex turn failed".into());
                self.terminal = true;
            }
            // Error events can describe a recoverable connection retry. Only
            // turn.failed or unsuccessful process exit determines failure.
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn finish(mut self, success: bool) -> RunOutcome {
        if !success || self.outcome.thread_id.is_none() || !self.terminal {
            self.outcome.status = RunStatus::Failed;
            self.outcome
                .error
                .get_or_insert_with(|| "Codex exited without a successful turn".into());
        }
        self.outcome.accounting_incomplete = self.outcome.usage.is_none();
        self.outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_and_failed_streams_never_succeed() {
        assert_eq!(
            EventReader::default().finish(true).status,
            RunStatus::Failed
        );
        let mut reader = EventReader::default();
        reader
            .accept(br#"{"type":"turn.failed","error":{"message":"secret"}}"#)
            .unwrap();
        let result = reader.finish(true);
        assert_eq!(result.status, RunStatus::Failed);
        assert!(result.accounting_incomplete);
        assert!(!result.error.unwrap().contains("secret"));
    }

    #[test]
    fn duplicate_usage_and_invalid_counts_are_rejected() {
        let event = br#"{"type":"turn.completed","usage":{"input_tokens":8,"cached_input_tokens":5,"output_tokens":3}}"#;
        let mut reader = EventReader::default();
        reader.accept(event).unwrap();
        assert!(reader.accept(event).is_err());
        assert!(
            Usage {
                input_tokens: 1,
                cached_input_tokens: 2,
                ..Usage::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Usage {
                output_tokens: 1,
                reasoning_output_tokens: 2,
                ..Usage::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Usage {
                input_tokens: 1,
                cache_write_input_tokens: 2,
                ..Usage::default()
            }
            .validate()
            .is_err()
        );
    }

    fn replay(transcript: &str) -> RunOutcome {
        let mut reader = EventReader::default();
        for line in transcript.lines().filter(|line| !line.trim().is_empty()) {
            reader.accept(line.as_bytes()).expect(line);
        }
        reader.finish(true)
    }

    /// Recorded from codex-cli 0.153.4 with the argument vector this crate
    /// builds, so the parser stays pinned to a real transcript rather than to
    /// an assumed schema.
    #[test]
    fn recorded_codex_transcripts_parse_into_complete_outcomes() {
        let outcome = replay(include_str!("../tests/fixtures/codex-0.153.4-exec.jsonl"));
        assert_eq!(outcome.status, RunStatus::Completed);
        assert_eq!(
            outcome.thread_id.as_deref(),
            Some("01a078bd-ba50-7580-9feb-6f3e968732d8")
        );
        assert_eq!(outcome.message, "pong");
        assert!(outcome.changes.is_empty());
        assert!(!outcome.accounting_incomplete);
        assert_eq!(
            outcome.usage,
            Some(Usage {
                input_tokens: 12102,
                cached_input_tokens: 0,
                cache_write_input_tokens: 0,
                output_tokens: 5,
                reasoning_output_tokens: 0,
            })
        );

        // The resumed turn repeats the same thread id, which the reader must
        // accept rather than treat as a switched session.
        let mut reader = EventReader::default();
        reader.outcome.thread_id = Some("01a078bd-ba50-7580-9feb-6f3e968732d8".into());
        for line in include_str!("../tests/fixtures/codex-0.153.4-exec-resume.jsonl")
            .lines()
            .filter(|line| !line.trim().is_empty())
        {
            reader.accept(line.as_bytes()).expect(line);
        }
        let resumed = reader.finish(true);
        assert_eq!(resumed.status, RunStatus::Completed);
        assert_eq!(
            resumed.thread_id.as_deref(),
            Some("01a078bd-ba50-7580-9feb-6f3e968732d8")
        );
        // Read-only sandbox held across the resume: the model reported it could
        // not write, and no file_change item was emitted.
        assert!(resumed.message.contains("read-only"));
        assert!(resumed.changes.is_empty());
        assert_eq!(resumed.usage.unwrap().reasoning_output_tokens, 35);
    }

    /// `file_change` is the one item shape the read-only live runs could not
    /// produce; the field names come from Codex's `FileUpdateChange`.
    #[test]
    fn completed_file_changes_are_collected_and_partial_ones_ignored() {
        let outcome = replay(concat!(
            r#"{"type":"thread.started","thread_id":"01a078bd-ba50-7580-9feb-6f3e968732d8"}"#,
            "\n",
            r#"{"type":"item.started","item":{"id":"item_0","type":"file_change","changes":[{"path":"a.rs","kind":"add"}],"status":"in_progress"}}"#,
            "\n",
            r#"{"type":"item.completed","item":{"id":"item_0","type":"file_change","changes":[{"path":"a.rs","kind":"add"},{"path":"b.rs","kind":"update"}],"status":"completed"}}"#,
            "\n",
            r#"{"type":"item.completed","item":{"id":"item_1","type":"file_change","changes":[{"path":"c.rs","kind":"delete"}],"status":"failed"}}"#,
            "\n",
            r#"{"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":2,"cache_write_input_tokens":3,"output_tokens":4,"reasoning_output_tokens":1}}"#,
            "\n",
        ));
        assert_eq!(outcome.status, RunStatus::Completed);
        let paths: Vec<_> = outcome.changes.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["a.rs", "b.rs"]);
        assert_eq!(outcome.changes[1].kind, "update");
        assert_eq!(outcome.usage.unwrap().cache_write_input_tokens, 3);
    }
}
