use std::collections::VecDeque;
use std::fmt::Write;

/// Sliding window memory for recent tool calls and responses.
/// Domain-agnostic: works with any tool that uses `Action.Type` convention.
#[derive(Debug, Clone, Default)]
pub struct ToolMemory {
    entries: VecDeque<ToolMemoryEntry>,
    max_entries: usize,
    /// Full text of the most recent Observe result (untruncated for state broadcast)
    last_observe_full: Option<String>,
    /// Tracks consecutive same-type actions across ticks (persists beyond window)
    last_action_type: Option<String>,
    consecutive_same_type: usize,
}

#[derive(Debug, Clone)]
pub struct ToolMemoryEntry {
    pub tool_name: String,
    pub action_type: String,
    pub action_key: String,
    pub args_summary: String,
    pub result_summary: String,
    pub is_error: bool,
    pub is_duplicate: bool,
    pub is_observe: bool,
    pub timestamp: std::time::Instant,
}

impl ToolMemory {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(max_entries),
            max_entries,
            last_observe_full: None,
            last_action_type: None,
            consecutive_same_type: 0,
        }
    }

    /// Clear all entries and reset consecutive action tracking.
    /// Used when the orchestrator detects degenerate output and needs
    /// to start fresh without poisoned context.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.last_action_type = None;
        self.consecutive_same_type = 0;
        // Preserve last_observe_full — the colony state is still valid
    }

    pub fn add(&mut self, tool_name: String, args: &serde_json::Value, result: &str) {
        let action_type = args
            .get("Action")
            .and_then(|a| a.get("Type"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();

        // Store the full result of any read-only observation tool for state broadcast.
        // Convention: tools named "*:observe" or "*:state" are observation endpoints.
        let is_observe = tool_name.ends_with(":observe")
            || tool_name.ends_with(":state")
            || tool_name.ends_with(":status");
        if is_observe {
            self.last_observe_full = Some(result.to_string());
        } else if !action_type.is_empty() {
            // Track consecutive same-type actions across ticks
            if self.last_action_type.as_deref() == Some(&action_type) {
                self.consecutive_same_type += 1;
            } else {
                self.last_action_type = Some(action_type.clone());
                self.consecutive_same_type = 1;
            }
        }

        let action_key = Self::extract_action_key(args);

        let is_error = result.contains("\"Error\"") || result.starts_with("Error:");

        let is_duplicate =
            !action_key.is_empty() && self.entries.iter().any(|e| e.action_key == action_key);

        let args_summary: String = serde_json::to_string(args)
            .unwrap_or_default()
            .chars()
            .take(100)
            .collect();
        // Error results get more space so actionable corrections aren't truncated
        // (e.g. "Unknown building 'X'. Examples: Bed, SimpleResearchBench, ...")
        let result_limit = if is_error { 400 } else { 200 };
        let result_summary: String = result.chars().take(result_limit).collect();

        if self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }
        self.entries.push_back(ToolMemoryEntry {
            tool_name,
            action_type,
            action_key,
            args_summary,
            result_summary,
            is_error,
            is_duplicate,
            is_observe,
            timestamp: std::time::Instant::now(),
        });
    }

    pub fn format_for_prompt(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }
        let mut output = String::from("\n\n## Recent Actions\n");
        for (i, entry) in self.entries.iter().enumerate() {
            let status = if entry.is_error { "FAILED" } else { "OK" };
            let label = if !entry.action_type.is_empty() {
                &entry.action_type
            } else {
                &entry.tool_name
            };
            let dup_warning = if entry.is_duplicate {
                " ← DUPLICATE (already done, try different params)"
            } else {
                ""
            };
            let _ = writeln!(
                output,
                "{}. {}: {}{}\n   → {}",
                i + 1,
                label,
                status,
                dup_warning,
                entry.result_summary
            );
        }
        output
    }

    /// Full text of the most recent Observe result (for state broadcast to specialists)
    pub fn last_observe_full(&self) -> Option<&str> {
        self.last_observe_full.as_deref()
    }

    /// Check if any recent entry was a meaningful action (has Action.Type, not observation)
    pub fn had_meaningful_action(&self) -> bool {
        self.entries
            .iter()
            .any(|e| !e.action_type.is_empty() && !e.is_observe)
    }

    /// Returns distinct action types used in recent entries (excluding observations)
    pub fn recent_action_types(&self) -> Vec<String> {
        let mut seen = Vec::new();
        for entry in self.entries.iter().rev() {
            if !entry.action_type.is_empty()
                && !entry.is_observe
                && !seen.contains(&entry.action_type)
            {
                seen.push(entry.action_type.clone());
            }
        }
        seen
    }

    /// Returns a warning if the same action type has been used 3+ times consecutively.
    /// Uses a persistent counter that tracks across ticks (not limited by sliding window).
    pub fn action_variety_warning(&self) -> String {
        if self.consecutive_same_type >= 3 {
            let action_type = self.last_action_type.as_deref().unwrap_or("unknown");
            format!(
                "\n\nACTION VARIETY: You have used {action_type} {} times in a row. \
                 This is unlikely to help. Try a DIFFERENT action type.\n",
                self.consecutive_same_type
            )
        } else {
            String::new()
        }
    }

    /// Format recent error entries as correction instructions for the next prompt.
    /// Returns up to 5 deduplicated errors so the model can learn from mistakes.
    pub fn format_errors_for_prompt(&self) -> String {
        let mut seen = std::collections::HashSet::new();
        let errors: Vec<_> = self
            .entries
            .iter()
            .rev()
            .filter(|e| e.is_error)
            .filter(|e| seen.insert(e.action_key.clone()))
            .take(5)
            .collect();

        if errors.is_empty() {
            return String::new();
        }

        let mut output = String::from("\n\n## Error Corrections (fix these on next step)\n");
        for entry in &errors {
            let label = if !entry.action_type.is_empty() {
                &entry.action_type
            } else {
                &entry.tool_name
            };
            use std::fmt::Write;
            let _ = writeln!(output, "- {label}: {}", entry.result_summary);
        }
        output
    }

    /// Build a dedup key from Action object fields.
    /// Generic: uses action type + sorted key=value pairs from all Action fields.
    fn extract_action_key(args: &serde_json::Value) -> String {
        let action = match args.get("Action") {
            Some(a) => a,
            None => return String::new(),
        };
        let action_type = action.get("Type").and_then(|t| t.as_str()).unwrap_or("");
        if action_type.is_empty() {
            return String::new();
        }

        let obj = match action.as_object() {
            Some(o) => o,
            None => return action_type.to_string(),
        };

        let mut params: Vec<String> = obj
            .iter()
            .filter(|(k, _)| *k != "Type")
            .map(|(k, v)| {
                let val = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                format!("{k}={val}")
            })
            .collect();
        params.sort();

        if params.is_empty() {
            action_type.to_string()
        } else {
            format!("{action_type}({})", params.join(","))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;
    use serde_json::json;

    #[spec("SRV-003")]
    #[test]
    fn test_action_key_with_params() {
        let args =
            json!({"Action": {"Type": "PlaceBlueprint", "ThingDef": "Bed", "X": 10, "Y": 15}});
        let key = ToolMemory::extract_action_key(&args);
        assert!(key.starts_with("PlaceBlueprint("));
        assert!(key.contains("ThingDef=Bed"));
        assert!(key.contains("X=10"));
        assert!(key.contains("Y=15"));
    }

    #[spec("SRV-003")]
    #[test]
    fn test_action_key_single_param() {
        let args = json!({"Action": {"Type": "CreateGrowingZone", "ZoneId": "food1"}});
        assert_eq!(
            ToolMemory::extract_action_key(&args),
            "CreateGrowingZone(ZoneId=food1)"
        );
    }

    #[spec("SRV-003")]
    #[test]
    fn test_action_key_type_only_no_params() {
        // Action types without extra params produce a plain key (no parens)
        let args = json!({"Action": {"Type": "Observe"}});
        assert_eq!(ToolMemory::extract_action_key(&args), "Observe");
    }

    #[spec("SRV-003")]
    #[test]
    fn test_action_key_no_action_is_empty() {
        let args = json!({"task": "something"});
        assert_eq!(ToolMemory::extract_action_key(&args), "");
    }

    #[spec("SRV-003")]
    #[test]
    fn test_action_key_no_params() {
        let args = json!({"Action": {"Type": "DoSomething"}});
        assert_eq!(ToolMemory::extract_action_key(&args), "DoSomething");
    }

    #[spec("SRV-003")]
    #[test]
    fn test_duplicate_detection() {
        let mut mem = ToolMemory::new(10);
        let args = json!({"Action": {"Type": "Build", "Item": "Wall", "X": 10, "Y": 10}});

        mem.add("tool_a".into(), &args, r#"{"ok":true}"#);
        assert!(!mem.entries.back().unwrap().is_duplicate);

        mem.add("tool_a".into(), &args, r#"{"ok":true}"#);
        assert!(mem.entries.back().unwrap().is_duplicate);
    }

    #[spec("SRV-003")]
    #[test]
    fn test_no_false_duplicate_different_params() {
        let mut mem = ToolMemory::new(10);
        let args1 = json!({"Action": {"Type": "Build", "X": 10, "Y": 10}});
        let args2 = json!({"Action": {"Type": "Build", "X": 12, "Y": 10}});

        mem.add("tool_a".into(), &args1, r#"{"ok":true}"#);
        mem.add("tool_a".into(), &args2, r#"{"ok":true}"#);
        assert!(!mem.entries.back().unwrap().is_duplicate);
    }

    #[spec("SRV-003")]
    #[test]
    fn test_format_shows_duplicate_warning() {
        let mut mem = ToolMemory::new(10);
        let args = json!({"Action": {"Type": "Build", "X": 10, "Y": 10}});

        mem.add("tool_a".into(), &args, r#"{"ok":true}"#);
        mem.add("tool_a".into(), &args, r#"{"ok":true}"#);

        let prompt = mem.format_for_prompt();
        assert!(prompt.contains("DUPLICATE"));
        assert!(prompt.contains("Build"));
    }

    #[spec("SRV-003")]
    #[test]
    fn test_had_meaningful_action() {
        let mut mem = ToolMemory::new(10);

        // No entries
        assert!(!mem.had_meaningful_action());

        // Observation tool (name ends in :observe) doesn't count
        let observe_args = json!({"Include": ["resources"]});
        mem.add("env:observe".into(), &observe_args, r#"{"ok":true}"#);
        assert!(!mem.had_meaningful_action());

        // Action counts (any tool name)
        let action = json!({"Action": {"Type": "DoWork", "Target": "item1"}});
        mem.add("any_tool".into(), &action, r#"{"ok":true}"#);
        assert!(mem.had_meaningful_action());

        // Action followed by observe still returns true
        mem.add("env:observe".into(), &observe_args, r#"{"ok":true}"#);
        assert!(mem.had_meaningful_action());

        // Delegation (no Action.Type) after action: action still in window
        let task = json!({"agent_id": "helper", "task": "assist"});
        mem.add("send_task".into(), &task, r#"{"ok":true}"#);
        assert!(mem.had_meaningful_action());

        // Fresh memory with only delegation: no meaningful action
        let mut mem2 = ToolMemory::new(10);
        mem2.add("send_task".into(), &task, r#"{"ok":true}"#);
        assert!(!mem2.had_meaningful_action());

        // Fresh memory with only observation: no meaningful action
        let mut mem3 = ToolMemory::new(10);
        mem3.add("env:observe".into(), &observe_args, r#"{"ok":true}"#);
        assert!(!mem3.had_meaningful_action());
    }

    #[spec("SRV-003")]
    #[test]
    fn test_action_variety_warning_triggers() {
        let mut mem = ToolMemory::new(10);
        let args1 = json!({"Action": {"Type": "Build", "Id": "1"}});
        let args2 = json!({"Action": {"Type": "Build", "Id": "2"}});
        let args3 = json!({"Action": {"Type": "Build", "Id": "3"}});

        mem.add("tool_a".into(), &args1, r#"{"ok":true}"#);
        mem.add("tool_a".into(), &args2, r#"{"ok":true}"#);
        assert!(mem.action_variety_warning().is_empty());

        mem.add("tool_a".into(), &args3, r#"{"ok":true}"#);
        let warning = mem.action_variety_warning();
        assert!(warning.contains("Build"));
        assert!(warning.contains("3 times"));
    }

    #[spec("SRV-003")]
    #[test]
    fn test_action_variety_ignores_observe() {
        let mut mem = ToolMemory::new(10);

        // Observation tool calls (matching :observe convention) should not
        // reset or affect the consecutive action type tracker.
        mem.add(
            "tool_a".into(),
            &json!({"Action": {"Type": "Build", "Id": "1"}}),
            "ok",
        );
        mem.add("env:observe".into(), &json!({}), "state");
        mem.add(
            "tool_a".into(),
            &json!({"Action": {"Type": "Build", "Id": "2"}}),
            "ok",
        );
        mem.add("env:observe".into(), &json!({}), "state");
        mem.add(
            "tool_a".into(),
            &json!({"Action": {"Type": "Build", "Id": "3"}}),
            "ok",
        );

        let warning = mem.action_variety_warning();
        assert!(warning.contains("Build"));
        assert!(warning.contains("3 times"));
    }

    #[spec("SRV-003")]
    #[test]
    fn test_action_variety_no_warning_with_mixed() {
        let mut mem = ToolMemory::new(10);
        mem.add(
            "tool_a".into(),
            &json!({"Action": {"Type": "Build", "Id": "1"}}),
            "ok",
        );
        mem.add(
            "tool_a".into(),
            &json!({"Action": {"Type": "Research", "Topic": "t1"}}),
            "ok",
        );
        mem.add(
            "tool_a".into(),
            &json!({"Action": {"Type": "Hunt", "Target": "a1"}}),
            "ok",
        );

        assert!(mem.action_variety_warning().is_empty());
    }

    #[spec("SRV-003")]
    #[test]
    fn test_recent_action_types() {
        let mut mem = ToolMemory::new(10);
        // Observation tool calls are excluded from action types
        mem.add("env:observe".into(), &json!({}), "ok");
        assert!(mem.recent_action_types().is_empty());

        mem.add(
            "tool_a".into(),
            &json!({"Action": {"Type": "Build", "Id": "1"}}),
            "ok",
        );
        mem.add(
            "tool_b".into(),
            &json!({"Action": {"Type": "Hunt", "Target": "a1"}}),
            "ok",
        );
        let types = mem.recent_action_types();
        assert_eq!(types.len(), 2);
        assert!(types.contains(&"Hunt".to_string()));
        assert!(types.contains(&"Build".to_string()));
    }

    #[spec("SRV-003")]
    #[test]
    fn test_last_observe_full_captured() {
        let mut mem = ToolMemory::new(10);
        assert!(mem.last_observe_full().is_none());

        // Tools ending in :observe store full results for state broadcast
        let args = json!({"Include": ["colonists", "resources", "alerts"]});
        let full_result = r#"{"state":{"tick":100,"resources":{"wood":50}}}"#;
        mem.add("game-rl:observe".into(), &args, full_result);
        assert_eq!(mem.last_observe_full(), Some(full_result));

        // Non-observe action doesn't overwrite
        let action = json!({"Action": {"Type": "Build", "Id": "1"}});
        mem.add("tool_a".into(), &action, r#"{"ok":true}"#);
        assert_eq!(mem.last_observe_full(), Some(full_result));

        // New observe overwrites
        let new_result = r#"{"state":{"tick":101,"resources":{"wood":45}}}"#;
        mem.add("env:observe".into(), &json!({}), new_result);
        assert_eq!(mem.last_observe_full(), Some(new_result));
    }

    #[spec("SRV-003")]
    #[test]
    fn test_last_observe_full_state_and_status_tools() {
        let mut mem = ToolMemory::new(10);

        // Tools ending in :state or :status also count as observations
        let result = r#"{"status":"running","health":0.95}"#;
        mem.add("system:state".into(), &json!({}), result);
        assert_eq!(mem.last_observe_full(), Some(result));

        let result2 = r#"{"active":true}"#;
        mem.add("agent:status".into(), &json!({}), result2);
        assert_eq!(mem.last_observe_full(), Some(result2));
    }

    #[spec("SRV-003")]
    #[test]
    fn test_error_detection() {
        let mut mem = ToolMemory::new(10);
        let args = json!({"Action": {"Type": "Build", "Item": "Wall"}});

        mem.add("tool_a".into(), &args, r#"{"Error":"No resources"}"#);
        assert!(mem.entries.back().unwrap().is_error);

        mem.add("tool_a".into(), &args, r#"{"ok":true}"#);
        assert!(!mem.entries.back().unwrap().is_error);
    }
}
