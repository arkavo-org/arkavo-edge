use anyhow::Result;
use arkavo_llm::Message;
use arkavo_router::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{Duration, interval};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthBatch {
    pub commands: Vec<CommandHealthData>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandHealthData {
    pub id: u32,
    pub operation: String,
    pub selector: String,
    pub duration_ms: u64,
    pub component: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutAnalysis {
    pub should_timeout: bool,
    pub user_message: Option<String>,
    pub severity: String,
    pub reasoning: String,
}

pub struct CommandHealthCollector {
    batch_interval: Duration,
}

impl CommandHealthCollector {
    pub fn new(batch_interval_secs: u64) -> Self {
        Self {
            batch_interval: Duration::from_secs(batch_interval_secs),
        }
    }

    pub async fn start(
        self,
        llm_tx: mpsc::Sender<HealthBatch>,
    ) -> Result<tokio::task::JoinHandle<()>> {
        let handle = tokio::spawn(async move {
            let mut batch_interval = interval(self.batch_interval);

            loop {
                batch_interval.tick().await;

                let batch = self.collect_batch().await;

                println!(
                    "[HEARTBEAT] CommandHealthCollector: {} commands in batch",
                    batch.commands.len()
                );

                if !batch.commands.is_empty() {
                    println!(
                        "[HEARTBEAT] Stuck commands detected: {:?}",
                        batch
                            .commands
                            .iter()
                            .map(|c| format!("id={} {}ms", c.id, c.duration_ms))
                            .collect::<Vec<_>>()
                    );
                    if let Err(e) = llm_tx.send(batch).await {
                        eprintln!("Failed to send health batch: {e}");
                    }
                } else {
                    println!("[HEARTBEAT] No stuck commands");
                }
            }
        });

        Ok(handle)
    }

    async fn collect_batch(&self) -> HealthBatch {
        use arkavo_observability::health_reporter::HealthRegistry;

        let registry = HealthRegistry::global();
        let reports = registry.check_all().await;

        println!(
            "[HEARTBEAT] Health reports from registry: {} total",
            reports.len()
        );
        for report in &reports {
            println!(
                "[HEARTBEAT]   - {}: {} ({:?})",
                report.component, report.message, report.status
            );
        }

        let commands: Vec<CommandHealthData> = reports
            .iter()
            .filter_map(|report| {
                if report.component == "cef" {
                    let parsed = Self::parse_cef_health(&report.message, &report.component);
                    if parsed.is_some() {
                        println!("[HEARTBEAT] Parsed CEF health data: {:?}", parsed);
                    } else {
                        println!(
                            "[HEARTBEAT] Failed to parse CEF message: '{}'",
                            report.message
                        );
                    }
                    parsed
                } else {
                    None
                }
            })
            .collect();

        HealthBatch {
            commands,
            timestamp: chrono::Utc::now(),
        }
    }

    fn parse_cef_health(message: &str, component: &str) -> Option<CommandHealthData> {
        if message.contains("Stuck commands:") {
            let parts: Vec<&str> = message.split("Stuck commands: ").collect();
            if parts.len() < 2 {
                return None;
            }

            let command_info = parts[1];
            let info_parts: Vec<&str> = command_info.split_whitespace().collect();

            if info_parts.len() >= 5 && info_parts[0] == "Command" {
                let id = info_parts[1].parse::<u32>().ok()?;
                let operation = info_parts[2]
                    .trim_matches(|c| c == '(' || c == ')')
                    .to_string();
                let duration_ms = info_parts[4].trim_end_matches("ms").parse::<u64>().ok()?;
                let selector = info_parts
                    .get(6)?
                    .trim_matches(|c| c == '\'' || c == '"')
                    .to_string();

                return Some(CommandHealthData {
                    id,
                    operation,
                    selector,
                    duration_ms,
                    component: component.to_string(),
                });
            }
        }
        None
    }
}

pub struct TimeoutAnalyzer {
    router: Arc<Router>,
}

impl TimeoutAnalyzer {
    pub async fn new() -> Result<Self> {
        Ok(Self {
            router: Arc::new(Router::new().await?),
        })
    }

    #[cfg(test)]
    pub async fn new_offline() -> Result<Self> {
        Ok(Self {
            router: Arc::new(Router::new_offline().await?),
        })
    }

    pub async fn analyze_batch(&self, batch: HealthBatch) -> Result<Vec<TimeoutAnalysis>> {
        let mut analyses = Vec::new();

        for cmd in batch.commands {
            let analysis = self.analyze_command(&cmd).await?;
            analyses.push(analysis);
        }

        Ok(analyses)
    }

    async fn analyze_command(&self, cmd: &CommandHealthData) -> Result<TimeoutAnalysis> {
        let provider = self.router.get_local_provider();

        let prompt = self.build_timeout_prompt(cmd);

        let messages = vec![Message::user(prompt)];

        let response = provider.complete_with_options(messages, Some(150)).await?;

        self.parse_analysis(&response)
    }

    fn build_timeout_prompt(&self, cmd: &CommandHealthData) -> String {
        format!(
            r#"A CEF DOM command has been waiting:

ID: {}
Operation: {}
Selector: '{}'
Duration: {}ms

Should this timeout? Return JSON:
{{
  "should_timeout": true/false,
  "user_message": "Brief explanation for user" or null,
  "severity": "info"|"warning"|"critical",
  "reasoning": "Why timeout or not"
}}

Guidelines:
- Under 10s: Usually normal, don't timeout
- 10-30s: Warn user but keep waiting
- Over 30s: Timeout, likely stuck

JSON:"#,
            cmd.id, cmd.operation, cmd.selector, cmd.duration_ms
        )
    }

    fn parse_analysis(&self, response: &str) -> Result<TimeoutAnalysis> {
        let cleaned = response
            .chars()
            .filter(|c| !c.is_control() || *c == '\n' || *c == '\r' || *c == '\t')
            .collect::<String>();

        let json_str = if let Some(start) = cleaned.find('{') {
            let after_start = &cleaned[start..];
            let mut depth = 0;
            let mut end_pos = 0;

            for (i, ch) in after_start.chars().enumerate() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end_pos = i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }

            if end_pos > 0 {
                &after_start[..end_pos]
            } else {
                cleaned.trim()
            }
        } else {
            cleaned.trim()
        };

        serde_json::from_str(json_str)
            .map_err(|e| anyhow::anyhow!("Failed to parse timeout analysis: {e}"))
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cef_health_stuck_command() {
        let message = "Stuck commands: Command 2 (ReplaceInnerHTML) waiting 5200ms on '#content'";
        let data = CommandHealthCollector::parse_cef_health(message, "cef");

        assert!(data.is_some());
        let data = data.unwrap();
        assert_eq!(data.id, 2);
        assert_eq!(data.operation, "ReplaceInnerHTML");
        assert_eq!(data.duration_ms, 5200);
        assert_eq!(data.selector, "#content");
    }

    #[test]
    fn test_parse_cef_health_no_stuck_commands() {
        let message = "No active commands";
        let data = CommandHealthCollector::parse_cef_health(message, "cef");

        assert!(data.is_none());
    }

    #[test]
    fn test_parse_timeout_analysis() {
        let json_response = r#"{"should_timeout": true, "user_message": "Command stuck", "severity": "warning", "reasoning": "Over 30s"}"#;

        // Test only parsing logic, no router needed
        let analysis: TimeoutAnalysis = serde_json::from_str(json_response).unwrap();
        assert!(analysis.should_timeout);
        assert_eq!(analysis.severity, "warning");
        assert_eq!(analysis.user_message.as_deref(), Some("Command stuck"));
        assert_eq!(analysis.reasoning, "Over 30s");
    }
}
