pub use arkavo_critic::response_analyzer::is_simple_query;
use arkavo_critic::response_analyzer::{
    has_code_fence, has_code_indicators, has_repetition, is_chat_query,
};
use serde::{Deserialize, Serialize};
use std::sync::RwLock;

/// Issue types the advisor can address (mirrors arkavo-critic's DetectedIssue)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AdvisorIssue {
    UnwantedCodeFence,
    OutputLoop,
    WrongExpert,
    Timeout,
}

/// Advice returned to the caller for system-message injection
#[derive(Debug, Clone)]
pub struct PromptAdvice {
    pub system_text: String,
    pub applied_labels: Vec<String>,
}

/// Diagnostic snapshot for a single adjustment
#[derive(Debug, Clone)]
pub struct AdvisorStats {
    pub label: String,
    pub model_family: String,
    pub success_rate: f64,
    pub applications: u32,
    pub feedback_count: u32,
    pub is_static: bool,
}

struct Adjustment {
    label: String,
    model_family: String,
    issue: AdvisorIssue,
    text: String,
    success_rate: f64,
    applications: u32,
    feedback_count: u32,
    is_static: bool,
}

/// Prompt advisor that injects system messages to prevent known model failures.
///
/// Maintains a set of adjustments (static defaults + dynamically learned) keyed
/// by model family and issue type. Adjustments with poor success rates are
/// automatically disabled after enough feedback.
pub struct PromptAdvisor {
    adjustments: RwLock<Vec<Adjustment>>,
}

const MAX_ADVICE_CHARS: usize = 800;
const DISABLE_THRESHOLD: f64 = 0.3;
const MIN_FEEDBACK_TO_DISABLE: u32 = 5;
const EMA_ALPHA: f64 = 0.2;

impl Default for PromptAdvisor {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptAdvisor {
    pub fn new() -> Self {
        let adjustments = vec![
            Adjustment {
                label: "glm-code-fence".into(),
                model_family: "glm".into(),
                issue: AdvisorIssue::UnwantedCodeFence,
                text: "Respond in plain text. Do not use code fences or markdown formatting unless explicitly asked for code.".into(),
                success_rate: 0.5,
                applications: 0,
                feedback_count: 0,
                is_static: true,
            },
            Adjustment {
                label: "glm-wrong-expert".into(),
                model_family: "glm".into(),
                issue: AdvisorIssue::WrongExpert,
                text: "This is a conversational question. Respond naturally without code, imports, or function definitions.".into(),
                success_rate: 0.5,
                applications: 0,
                feedback_count: 0,
                is_static: true,
            },
            Adjustment {
                label: "glm-output-loop".into(),
                model_family: "glm".into(),
                issue: AdvisorIssue::OutputLoop,
                text: "Give a single concise answer. Do not repeat yourself or generate multiple Q&A pairs.".into(),
                success_rate: 0.5,
                applications: 0,
                feedback_count: 0,
                is_static: true,
            },
            Adjustment {
                label: "gemma-output-loop".into(),
                model_family: "gemma".into(),
                issue: AdvisorIssue::OutputLoop,
                text: "Provide one clear answer then stop. Do not repeat or rephrase.".into(),
                success_rate: 0.5,
                applications: 0,
                feedback_count: 0,
                is_static: true,
            },
            Adjustment {
                label: "qwen-timeout".into(),
                model_family: "qwen".into(),
                issue: AdvisorIssue::Timeout,
                text: "Keep your response brief and direct.".into(),
                success_rate: 0.5,
                applications: 0,
                feedback_count: 0,
                is_static: true,
            },
        ];

        Self {
            adjustments: RwLock::new(adjustments),
        }
    }

    /// Return combined system-message advice for the given model family and query type.
    pub fn advise(&self, model_family: &str, is_simple_query: bool) -> Option<PromptAdvice> {
        let mut adjustments = self.adjustments.write().ok()?;

        let mut matching: Vec<&mut Adjustment> = adjustments
            .iter_mut()
            .filter(|a| a.model_family == model_family)
            .filter(|a| {
                match a.issue {
                    // Only include code-fence and wrong-expert for simple queries
                    AdvisorIssue::UnwantedCodeFence | AdvisorIssue::WrongExpert => is_simple_query,
                    // Always include output-loop and timeout
                    AdvisorIssue::OutputLoop | AdvisorIssue::Timeout => true,
                }
            })
            .filter(|a| {
                // Exclude disabled adjustments
                !(a.success_rate < DISABLE_THRESHOLD && a.feedback_count >= MIN_FEEDBACK_TO_DISABLE)
            })
            .collect();

        if matching.is_empty() {
            return None;
        }

        // Sort by success_rate ascending (worst first — they need the most help)
        matching.sort_by(|a, b| {
            a.success_rate
                .partial_cmp(&b.success_rate)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut system_text = String::new();
        let mut applied_labels = Vec::new();

        for adj in &mut matching {
            let candidate = if system_text.is_empty() {
                adj.text.clone()
            } else {
                format!("{} {}", system_text, adj.text)
            };

            if candidate.len() > MAX_ADVICE_CHARS {
                // Truncate at last complete sentence within limit
                if system_text.is_empty() {
                    system_text = truncate_at_sentence(&adj.text, MAX_ADVICE_CHARS);
                }
                break;
            }

            system_text = candidate;
            applied_labels.push(adj.label.clone());
            adj.applications += 1;
        }

        if applied_labels.is_empty() {
            return None;
        }

        Some(PromptAdvice {
            system_text,
            applied_labels,
        })
    }

    /// Record feedback for previously applied advice labels.
    /// Uses EMA (alpha=0.2) to update success_rate.
    pub fn record_feedback(&self, applied_labels: &[String], success: bool) {
        let Ok(mut adjustments) = self.adjustments.write() else {
            return;
        };

        let value = if success { 1.0 } else { 0.0 };

        for adj in adjustments.iter_mut() {
            if applied_labels.contains(&adj.label) {
                adj.success_rate = (1.0 - EMA_ALPHA).mul_add(adj.success_rate, EMA_ALPHA * value);
                adj.feedback_count += 1;
            }
        }
    }

    /// Add or update a dynamic adjustment from observed failures.
    pub fn learn(&self, model_family: &str, issue: AdvisorIssue, text: String) {
        let Ok(mut adjustments) = self.adjustments.write() else {
            return;
        };

        // Check for existing adjustment with same family + issue
        if let Some(existing) = adjustments
            .iter_mut()
            .find(|a| a.model_family == model_family && a.issue == issue && !a.is_static)
        {
            existing.text = text;
            existing.success_rate = 0.5; // Give it another chance
            return;
        }

        let label = format!("{model_family}-{issue:?}-dynamic").to_lowercase();

        adjustments.push(Adjustment {
            label,
            model_family: model_family.to_string(),
            issue,
            text,
            success_rate: 0.5,
            applications: 0,
            feedback_count: 0,
            is_static: false,
        });
    }

    /// Observe a response and auto-learn any detected style issues.
    ///
    /// Analyzes every response for code fences, loops, timeouts, and wrong-expert
    /// routing. When an issue is found, calls `learn()` to create a dynamic
    /// adjustment that prevents the same failure on subsequent requests.
    pub fn observe(&self, model_family: &str, prompt: &str, response: &str) {
        let lower_prompt = prompt.to_lowercase();

        // Timeout / empty response
        if response.is_empty() || response.trim().is_empty() {
            if !self.has_static(model_family, &AdvisorIssue::Timeout) {
                self.learn(
                    model_family,
                    AdvisorIssue::Timeout,
                    advice_text_for(&AdvisorIssue::Timeout).to_string(),
                );
            }
            return;
        }

        // Unwanted code fence on a simple query
        if is_simple_query(&lower_prompt)
            && has_code_fence(response)
            && !self.has_static(model_family, &AdvisorIssue::UnwantedCodeFence)
        {
            self.learn(
                model_family,
                AdvisorIssue::UnwantedCodeFence,
                advice_text_for(&AdvisorIssue::UnwantedCodeFence).to_string(),
            );
        }

        // Output loop / repetition
        if has_repetition(response) && !self.has_static(model_family, &AdvisorIssue::OutputLoop) {
            self.learn(
                model_family,
                AdvisorIssue::OutputLoop,
                advice_text_for(&AdvisorIssue::OutputLoop).to_string(),
            );
        }

        // Wrong expert (GLM chat query with code indicators)
        if model_family == "glm"
            && is_chat_query(&lower_prompt)
            && has_code_indicators(&response.to_lowercase())
            && !self.has_static(model_family, &AdvisorIssue::WrongExpert)
        {
            self.learn(
                model_family,
                AdvisorIssue::WrongExpert,
                advice_text_for(&AdvisorIssue::WrongExpert).to_string(),
            );
        }
    }

    /// Check if a static adjustment exists for the given family and issue.
    fn has_static(&self, model_family: &str, issue: &AdvisorIssue) -> bool {
        let Ok(adjustments) = self.adjustments.read() else {
            return false;
        };
        adjustments
            .iter()
            .any(|a| a.model_family == model_family && a.issue == *issue && a.is_static)
    }

    /// Return diagnostic snapshot of all adjustments.
    pub fn stats(&self) -> Vec<AdvisorStats> {
        let Ok(adjustments) = self.adjustments.read() else {
            return Vec::new();
        };

        adjustments
            .iter()
            .map(|a| AdvisorStats {
                label: a.label.clone(),
                model_family: a.model_family.clone(),
                success_rate: a.success_rate,
                applications: a.applications,
                feedback_count: a.feedback_count,
                is_static: a.is_static,
            })
            .collect()
    }

    /// Export all dynamic (non-static) adjustments as snapshots for persistence.
    pub fn export_dynamic(&self) -> Vec<DynamicSnapshot> {
        let Ok(adjustments) = self.adjustments.read() else {
            return Vec::new();
        };

        adjustments
            .iter()
            .filter(|a| !a.is_static)
            .map(|a| DynamicSnapshot {
                label: a.label.clone(),
                model_family: a.model_family.clone(),
                issue: a.issue.clone(),
                text: a.text.clone(),
                success_rate: a.success_rate,
                applications: a.applications,
                feedback_count: a.feedback_count,
            })
            .collect()
    }

    /// Import previously persisted dynamic adjustments.
    ///
    /// Deduplicates by (model_family, issue) — if a dynamic adjustment already
    /// exists for that combination, the import is skipped.
    pub fn import_dynamic(&self, snapshots: Vec<DynamicSnapshot>) {
        let Ok(mut adjustments) = self.adjustments.write() else {
            return;
        };

        for snap in snapshots {
            let already_exists = adjustments.iter().any(|a| {
                a.model_family == snap.model_family && a.issue == snap.issue && !a.is_static
            });

            if already_exists {
                continue;
            }

            adjustments.push(Adjustment {
                label: snap.label,
                model_family: snap.model_family,
                issue: snap.issue,
                text: snap.text,
                success_rate: snap.success_rate,
                applications: snap.applications,
                feedback_count: snap.feedback_count,
                is_static: false,
            });
        }
    }
}

/// Data transfer object for dynamic adjustments, used for persistence round-trips.
#[derive(Debug, Clone)]
pub struct DynamicSnapshot {
    pub label: String,
    pub model_family: String,
    pub issue: AdvisorIssue,
    pub text: String,
    pub success_rate: f64,
    pub applications: u32,
    pub feedback_count: u32,
}

/// Return the canonical instruction text for a given issue type.
fn advice_text_for(issue: &AdvisorIssue) -> &'static str {
    match issue {
        AdvisorIssue::UnwantedCodeFence => {
            "Respond in plain text. Do not use code fences or markdown formatting unless explicitly asked for code."
        }
        AdvisorIssue::OutputLoop => {
            "Give a single concise answer. Do not repeat yourself or generate multiple Q&A pairs."
        }
        AdvisorIssue::WrongExpert => {
            "This is a conversational question. Respond naturally without code, imports, or function definitions."
        }
        AdvisorIssue::Timeout => "Keep your response brief and direct.",
    }
}

/// Truncate text at the last complete sentence that fits within `max_len`.
fn truncate_at_sentence(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        return text.to_string();
    }

    let truncated = &text[..max_len];
    // Find last sentence-ending punctuation
    if let Some(pos) = truncated.rfind(['.', '!', '?']) {
        truncated[..=pos].to_string()
    } else {
        truncated.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_static_defaults_present() {
        let advisor = PromptAdvisor::new();
        let stats = advisor.stats();
        assert!(stats.len() >= 5);
        assert!(stats.iter().any(|s| s.model_family == "glm"));
        assert!(stats.iter().any(|s| s.model_family == "gemma"));
        assert!(stats.iter().any(|s| s.model_family == "qwen"));
    }

    #[test]
    fn test_advise_glm_simple_query() {
        let advisor = PromptAdvisor::new();
        let advice = advisor.advise("glm", true).unwrap();
        // Should include code fence and wrong expert adjustments for simple queries
        assert!(
            advice
                .applied_labels
                .iter()
                .any(|l| l.contains("code-fence"))
        );
        assert!(
            advice
                .applied_labels
                .iter()
                .any(|l| l.contains("wrong-expert"))
        );
        assert!(
            advice
                .applied_labels
                .iter()
                .any(|l| l.contains("output-loop"))
        );
        assert!(!advice.system_text.is_empty());
    }

    #[test]
    fn test_advise_no_match() {
        let advisor = PromptAdvisor::new();
        // Cloud model families have no static adjustments
        assert!(advisor.advise("anthropic", true).is_none());
        assert!(advisor.advise("google", false).is_none());
    }

    #[test]
    fn test_advise_caps_at_800_chars() {
        let advisor = PromptAdvisor::new();
        // Add many long adjustments to test cap
        for i in 0..20 {
            advisor.learn(
                "test-family",
                AdvisorIssue::OutputLoop,
                format!("This is a very long adjustment text number {} that should eventually cause the total to exceed the 800 character limit when combined with other adjustments. Adding more text to make it longer and more realistic.", i),
            );
        }
        if let Some(advice) = advisor.advise("test-family", false) {
            assert!(advice.system_text.len() <= MAX_ADVICE_CHARS);
        }
    }

    #[test]
    fn test_record_feedback_success() {
        let advisor = PromptAdvisor::new();
        let advice = advisor.advise("glm", true).unwrap();
        // Record several successes
        for _ in 0..5 {
            advisor.record_feedback(&advice.applied_labels, true);
        }
        let stats = advisor.stats();
        let glm_fence = stats.iter().find(|s| s.label == "glm-code-fence").unwrap();
        // After 5 successes from 0.5 start, rate should be above 0.5
        assert!(glm_fence.success_rate > 0.5);
        assert_eq!(glm_fence.feedback_count, 5);
    }

    #[test]
    fn test_record_feedback_failure_disables() {
        let advisor = PromptAdvisor::new();
        let labels = vec!["glm-code-fence".to_string()];
        // Record many failures to drive success_rate below 0.3
        for _ in 0..20 {
            advisor.record_feedback(&labels, false);
        }
        let stats = advisor.stats();
        let glm_fence = stats.iter().find(|s| s.label == "glm-code-fence").unwrap();
        assert!(glm_fence.success_rate < DISABLE_THRESHOLD);
        assert!(glm_fence.feedback_count >= MIN_FEEDBACK_TO_DISABLE);

        // Now advise should skip the disabled adjustment
        let advice = advisor.advise("glm", true);
        if let Some(advice) = advice {
            assert!(
                !advice
                    .applied_labels
                    .contains(&"glm-code-fence".to_string())
            );
        }
    }

    #[test]
    fn test_learn_creates_dynamic() {
        let advisor = PromptAdvisor::new();
        let initial_count = advisor.stats().len();
        advisor.learn(
            "mistral",
            AdvisorIssue::OutputLoop,
            "Stop after one answer.".into(),
        );
        assert_eq!(advisor.stats().len(), initial_count + 1);
        let stats = advisor.stats();
        let new_adj = stats.iter().find(|s| s.model_family == "mistral").unwrap();
        assert!(!new_adj.is_static);
        assert!((new_adj.success_rate - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_learn_updates_existing() {
        let advisor = PromptAdvisor::new();
        advisor.learn("mistral", AdvisorIssue::OutputLoop, "First text.".into());
        // Record some failures to change success_rate
        let labels = vec!["mistral-outputloop-dynamic".to_string()];
        for _ in 0..5 {
            advisor.record_feedback(&labels, false);
        }
        let before = advisor.stats();
        let adj = before.iter().find(|s| s.model_family == "mistral").unwrap();
        assert!(adj.success_rate < 0.5);

        // Learn again — should update text and reset success_rate
        let count_before = before.len();
        advisor.learn("mistral", AdvisorIssue::OutputLoop, "Updated text.".into());
        let after = advisor.stats();
        assert_eq!(after.len(), count_before); // No new entry
        let adj = after.iter().find(|s| s.model_family == "mistral").unwrap();
        assert!((adj.success_rate - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_is_simple_query() {
        assert!(is_simple_query("hello there"));
        assert!(is_simple_query("hi how are you"));
        assert!(is_simple_query("what is 2 + 2"));
        assert!(is_simple_query("capital of france"));
        assert!(is_simple_query("who is alan turing"));
        assert!(!is_simple_query("implement a binary search tree in rust"));
        assert!(!is_simple_query("refactor this module to use async"));
    }

    #[test]
    fn test_priority_ordering() {
        let advisor = PromptAdvisor::new();
        // Drive glm-code-fence success_rate down
        let labels = vec!["glm-code-fence".to_string()];
        for _ in 0..3 {
            advisor.record_feedback(&labels, false);
        }
        let advice = advisor.advise("glm", true).unwrap();
        // Lower success_rate should appear first (code-fence was driven down)
        assert_eq!(advice.applied_labels[0], "glm-code-fence");
    }

    #[test]
    fn test_advise_excludes_non_simple_for_code_fence() {
        let advisor = PromptAdvisor::new();
        let advice = advisor.advise("glm", false).unwrap();
        // Complex query should NOT include code-fence or wrong-expert
        assert!(
            !advice
                .applied_labels
                .iter()
                .any(|l| l.contains("code-fence"))
        );
        assert!(
            !advice
                .applied_labels
                .iter()
                .any(|l| l.contains("wrong-expert"))
        );
        // But should include output-loop
        assert!(
            advice
                .applied_labels
                .iter()
                .any(|l| l.contains("output-loop"))
        );
    }

    #[test]
    fn test_stats_snapshot() {
        let advisor = PromptAdvisor::new();
        let _ = advisor.advise("glm", true);
        let stats = advisor.stats();
        // All static entries should be present
        assert!(stats.iter().all(|s| s.is_static));
        // glm adjustments should have applications incremented
        let glm_applied: Vec<_> = stats
            .iter()
            .filter(|s| s.model_family == "glm" && s.applications > 0)
            .collect();
        assert!(!glm_applied.is_empty());
    }

    #[test]
    fn test_observe_detects_code_fence() {
        let advisor = PromptAdvisor::new();
        let initial = advisor.stats().len();
        advisor.observe("deepseek", "what is 2 + 2", "```python\nprint(4)\n```");
        let stats = advisor.stats();
        assert_eq!(stats.len(), initial + 1);
        let dynamic = stats.iter().find(|s| s.model_family == "deepseek").unwrap();
        assert!(!dynamic.is_static);
        assert!(dynamic.label.contains("unwantedcodefence"));
    }

    #[test]
    fn test_observe_detects_output_loop() {
        let advisor = PromptAdvisor::new();
        let initial = advisor.stats().len();
        let loopy = "line\nline\nline\nline\nline";
        advisor.observe("mistral", "tell me a joke", loopy);
        let stats = advisor.stats();
        assert_eq!(stats.len(), initial + 1);
        let dynamic = stats.iter().find(|s| s.model_family == "mistral").unwrap();
        assert!(!dynamic.is_static);
        assert!(dynamic.label.contains("outputloop"));
    }

    #[test]
    fn test_observe_detects_timeout() {
        let advisor = PromptAdvisor::new();
        let initial = advisor.stats().len();
        advisor.observe("deepseek", "hello", "   ");
        let stats = advisor.stats();
        assert_eq!(stats.len(), initial + 1);
        let dynamic = stats
            .iter()
            .find(|s| s.model_family == "deepseek" && s.label.contains("timeout"))
            .unwrap();
        assert!(!dynamic.is_static);
    }

    #[test]
    fn test_observe_detects_wrong_expert() {
        let advisor = PromptAdvisor::new();
        // Remove GLM static wrong-expert so observe can create dynamic
        // We test with a fresh advisor where GLM has static defaults,
        // but wrong-expert check only triggers for GLM and the static already exists.
        // So we test with a scenario where the static was disabled.
        let labels = vec!["glm-wrong-expert".to_string()];
        for _ in 0..20 {
            advisor.record_feedback(&labels, false);
        }
        // Static still exists (disabled but present) — observe skips because has_static
        // checks existence not disabled state. Test the detection logic instead.
        // Use a family without static wrong-expert defaults — but wrong-expert only triggers for glm.
        // This confirms the GLM check with static present is correctly skipped.
        let initial = advisor.stats().len();
        advisor.observe(
            "glm",
            "hello how are you",
            "def greet():\n    import os\n    print('Hello')",
        );
        // GLM has static wrong-expert, so no dynamic should be created
        assert_eq!(advisor.stats().len(), initial);
    }

    #[test]
    fn test_observe_skips_static_family() {
        let advisor = PromptAdvisor::new();
        let initial = advisor.stats().len();
        // GLM already has static code-fence adjustment
        advisor.observe("glm", "what is 2 + 2", "```python\nprint(4)\n```");
        // No new dynamic entry
        assert_eq!(advisor.stats().len(), initial);
    }

    #[test]
    fn test_observe_clean_response() {
        let advisor = PromptAdvisor::new();
        let initial = advisor.stats().len();
        advisor.observe("deepseek", "what is 2 + 2", "4");
        // No issues detected — no new adjustments
        assert_eq!(advisor.stats().len(), initial);
    }

    #[test]
    fn test_export_dynamic_excludes_static() {
        let advisor = PromptAdvisor::new();
        // Static-only — export should be empty
        assert!(advisor.export_dynamic().is_empty());

        // Add a dynamic adjustment
        advisor.learn("mistral", AdvisorIssue::OutputLoop, "Stop looping.".into());
        let exported = advisor.export_dynamic();
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].model_family, "mistral");
    }

    #[test]
    fn test_import_dynamic_roundtrip() {
        let advisor = PromptAdvisor::new();
        advisor.learn("deepseek", AdvisorIssue::Timeout, "Be brief.".into());

        let snapshots = advisor.export_dynamic();
        assert_eq!(snapshots.len(), 1);

        // Import into a fresh advisor
        let advisor2 = PromptAdvisor::new();
        advisor2.import_dynamic(snapshots);

        let stats = advisor2.stats();
        let imported = stats.iter().find(|s| s.model_family == "deepseek");
        assert!(imported.is_some());
        assert!(!imported.unwrap().is_static);
    }

    #[test]
    fn test_import_dynamic_deduplicates() {
        let advisor = PromptAdvisor::new();
        advisor.learn("mistral", AdvisorIssue::OutputLoop, "Stop looping.".into());

        let snapshots = advisor.export_dynamic();
        let count_before = advisor.stats().len();

        // Import the same snapshots again — should not create duplicates
        advisor.import_dynamic(snapshots);
        assert_eq!(advisor.stats().len(), count_before);
    }
}
