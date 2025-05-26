use crate::{Result, TestError};
use chrono::{DateTime, Utc};
use handlebars::Handlebars;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportData {
    pub title: String,
    pub timestamp: DateTime<Utc>,
    pub summary: Summary,
    pub scenarios: Vec<ScenarioReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub total_scenarios: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub duration: Duration,
    pub pass_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioReport {
    pub name: String,
    pub status: TestStatus,
    pub duration: Duration,
    pub steps: Vec<StepReport>,
    pub ai_insights: Option<String>,
    pub minimal_reproduction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepReport {
    pub keyword: String,
    pub text: String,
    pub status: TestStatus,
    pub error: Option<String>,
    pub screenshot: Option<String>,
    pub duration: Duration,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TestStatus {
    Passed,
    Failed,
    Skipped,
    Pending,
}

#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    Markdown,
    Html,
    Json,
    Slack,
}

pub struct BusinessReporter {
    template_engine: Handlebars<'static>,
    output_format: OutputFormat,
}

impl BusinessReporter {
    pub fn new(output_format: OutputFormat) -> Result<Self> {
        let mut template_engine = Handlebars::new();
        
        template_engine.register_template_string(
            "markdown",
            include_str!("../../templates/report_markdown.hbs")
        ).map_err(|e| TestError::Reporting(format!("Failed to register markdown template: {}", e)))?;
        
        template_engine.register_template_string(
            "html",
            include_str!("../../templates/report_html.hbs")
        ).map_err(|e| TestError::Reporting(format!("Failed to register HTML template: {}", e)))?;
        
        Ok(Self {
            template_engine,
            output_format,
        })
    }
    
    pub fn generate_report(&self, results: &[ScenarioResult]) -> Result<String> {
        let report_data = self.build_report_data(results);
        
        match self.output_format {
            OutputFormat::Markdown => self.render_markdown(&report_data),
            OutputFormat::Html => self.render_html(&report_data),
            OutputFormat::Json => self.render_json(&report_data),
            OutputFormat::Slack => self.render_slack(&report_data),
        }
    }
    
    fn build_report_data(&self, results: &[ScenarioResult]) -> ReportData {
        let total_scenarios = results.len();
        let passed = results.iter().filter(|r| r.status == TestStatus::Passed).count();
        let failed = results.iter().filter(|r| r.status == TestStatus::Failed).count();
        let skipped = results.iter().filter(|r| r.status == TestStatus::Skipped).count();
        
        let total_duration = results.iter()
            .map(|r| r.duration)
            .fold(Duration::from_secs(0), |acc, d| acc + d);
        
        let pass_rate = if total_scenarios > 0 {
            (passed as f64 / total_scenarios as f64) * 100.0
        } else {
            0.0
        };
        
        let summary = Summary {
            total_scenarios,
            passed,
            failed,
            skipped,
            duration: total_duration,
            pass_rate,
        };
        
        let scenarios = results.iter().map(|r| {
            ScenarioReport {
                name: r.name.clone(),
                status: r.status,
                duration: r.duration,
                steps: r.steps.iter().map(|s| StepReport {
                    keyword: s.keyword.clone(),
                    text: s.text.clone(),
                    status: s.status,
                    error: s.error.clone(),
                    screenshot: s.screenshot_path.clone(),
                    duration: s.duration,
                }).collect(),
                ai_insights: r.ai_analysis.clone(),
                minimal_reproduction: r.minimal_reproduction.clone(),
            }
        }).collect();
        
        ReportData {
            title: "Arkavo Edge Test Report".to_string(),
            timestamp: Utc::now(),
            summary,
            scenarios,
        }
    }
    
    fn render_markdown(&self, data: &ReportData) -> Result<String> {
        self.template_engine.render("markdown", data)
            .map_err(|e| TestError::Reporting(format!("Failed to render markdown: {}", e)))
    }
    
    fn render_html(&self, data: &ReportData) -> Result<String> {
        self.template_engine.render("html", data)
            .map_err(|e| TestError::Reporting(format!("Failed to render HTML: {}", e)))
    }
    
    fn render_json(&self, data: &ReportData) -> Result<String> {
        serde_json::to_string_pretty(data)
            .map_err(|e| TestError::Reporting(format!("Failed to render JSON: {}", e)))
    }
    
    fn render_slack(&self, data: &ReportData) -> Result<String> {
        let emoji = if data.summary.pass_rate >= 100.0 {
            "✅"
        } else if data.summary.pass_rate >= 80.0 {
            "⚠️"
        } else {
            "❌"
        };
        
        let message = format!(
            r#"{{
    "text": "{} *{}*",
    "blocks": [
        {{
            "type": "header",
            "text": {{
                "type": "plain_text",
                "text": "{}"
            }}
        }},
        {{
            "type": "section",
            "fields": [
                {{
                    "type": "mrkdwn",
                    "text": "*Total Scenarios:* {}"
                }},
                {{
                    "type": "mrkdwn",
                    "text": "*Pass Rate:* {:.1}%"
                }},
                {{
                    "type": "mrkdwn",
                    "text": "*Passed:* {} ✅"
                }},
                {{
                    "type": "mrkdwn",
                    "text": "*Failed:* {} ❌"
                }},
                {{
                    "type": "mrkdwn",
                    "text": "*Duration:* {:?}"
                }},
                {{
                    "type": "mrkdwn",
                    "text": "*Time:* {}"
                }}
            ]
        }}
    ]
}}"#,
            emoji,
            data.title,
            data.title,
            data.summary.total_scenarios,
            data.summary.pass_rate,
            data.summary.passed,
            data.summary.failed,
            data.summary.duration,
            data.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        );
        
        Ok(message)
    }
}

#[derive(Debug, Clone)]
pub struct ScenarioResult {
    pub name: String,
    pub status: TestStatus,
    pub duration: Duration,
    pub steps: Vec<StepResult>,
    pub ai_analysis: Option<String>,
    pub minimal_reproduction: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StepResult {
    pub keyword: String,
    pub text: String,
    pub status: TestStatus,
    pub error: Option<String>,
    pub screenshot_path: Option<String>,
    pub duration: Duration,
}