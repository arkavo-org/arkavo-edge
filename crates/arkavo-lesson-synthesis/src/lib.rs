//! The lesson-synthesis rules shared by every consolidation surface.
//!
//! A lesson must reason about which *actions* produce good outcomes, not
//! about tool-calling mechanics, so read-only observe/list/hash/summary
//! calls are stripped from evidence before synthesis. This crate is the
//! single definition of that rule — the runtime consolidation teacher and
//! the audit-plane shadow job both consume it, so the two planes cannot
//! drift apart on what counts as an action.
//!
//! Pure functions over std only: depending on this crate creates no
//! runtime contact.

/// A tool is an *action* tool only when its name does not mark it as a
/// read-only observe/list/hash/summary call.
#[must_use]
pub fn is_action_tool(tool: &str) -> bool {
    let lower = tool.to_lowercase();
    !lower.contains("observe")
        && !lower.contains("list")
        && !lower.contains("hash")
        && !lower.contains("summary")
}

/// Apply [`is_action_tool`] to a tool list — the evidence-stripping step.
#[must_use]
pub fn strip_observe(tools: &[String]) -> Vec<String> {
    tools
        .iter()
        .filter(|t| is_action_tool(t))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_names_are_not_actions() {
        for tool in [
            "game-rl:observe",
            "Observe",
            "list_files",
            "tools/list",
            "content_hash",
            "summary_report",
        ] {
            assert!(!is_action_tool(tool), "{tool} must be read-only");
        }
    }

    #[test]
    fn action_names_pass() {
        for tool in ["set_priority", "PlaceOrder", "game-rl:act", "deploy"] {
            assert!(is_action_tool(tool), "{tool} must be an action");
        }
    }

    #[test]
    fn strip_observe_keeps_only_actions_in_order() {
        let tools = vec![
            "observe".to_string(),
            "set_priority".to_string(),
            "list_files".to_string(),
            "deploy".to_string(),
        ];
        assert_eq!(strip_observe(&tools), vec!["set_priority", "deploy"]);
    }
}
