//! What one agent cycle is asked to do, and whether it should run at all.
//!
//! The loop serves two kinds of agent with one tick. An orchestrator acts on
//! its own, so an idle tick still runs. A specialist is passive: it spends
//! inference only on work someone sent it. Keeping those rules here lets them
//! be tested without a model.

use arkavo_protocol::agent_config::AgentMode;

/// Prompt of a cycle that has nothing new to work on.
pub(super) const IDLE_PROMPT: &str = "Continue.";

/// Keeps a toolless advisor's analysis short enough to act on.
const ADVISORY_BREVITY: &str =
    "Respond in under 200 words. No reasoning preamble. Give actionable advice only.";

/// Everything that can put work in front of a cycle.
pub(super) struct CycleInputs<'a> {
    pub specialist_context: &'a str,
    pub dead_man_warning: &'a str,
    pub message_block: &'a str,
    pub has_mcp_tools: bool,
}

/// Build the user prompt for a cycle.
pub(super) fn assemble(inputs: &CycleInputs<'_>) -> String {
    let msg_section = if inputs.message_block.is_empty() {
        String::new()
    } else {
        format!("\n\n## Incoming Messages\n{}", inputs.message_block)
    };
    let extra = format!(
        "{}{}{msg_section}",
        inputs.specialist_context, inputs.dead_man_warning
    );
    let body = extra.trim();
    if body.is_empty() {
        return IDLE_PROMPT.to_string();
    }
    if !inputs.has_mcp_tools {
        return format!("{body}\n\n{ADVISORY_BREVITY}");
    }
    body.to_string()
}

/// True when the cycle has nothing to work on and the agent is not one that
/// acts unprompted, so running it would only burn inference.
///
/// A specialist is passive from its first tick. A toolless orchestrator keeps
/// its first cycle: that is its one chance to act on its purpose before any
/// specialist has answered.
pub(super) fn is_idle(
    mode: &AgentMode,
    has_mcp_tools: bool,
    cycle: u64,
    cycle_prompt: &str,
    requester_waiting: bool,
) -> bool {
    if cycle_prompt != IDLE_PROMPT || requester_waiting {
        return false;
    }
    *mode == AgentMode::Specialist || (!has_mcp_tools && cycle > 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(message_block: &str) -> CycleInputs<'_> {
        CycleInputs {
            specialist_context: "",
            dead_man_warning: "",
            message_block,
            has_mcp_tools: false,
        }
    }

    #[test]
    fn nothing_to_work_on_is_the_idle_prompt() {
        assert_eq!(assemble(&inputs("")), IDLE_PROMPT);
    }

    #[test]
    fn incoming_messages_become_the_prompt() {
        let prompt = assemble(&inputs("[event from=game] raid"));
        assert!(
            prompt.starts_with("## Incoming Messages\n[event from=game] raid"),
            "got {prompt}"
        );
    }

    /// Regression: a specialist ran a full planner cycle on `Continue.` at
    /// startup, before any task had arrived.
    #[test]
    fn a_specialist_is_idle_from_its_first_cycle() {
        for has_mcp_tools in [false, true] {
            for cycle in [1, 2, 50] {
                assert!(
                    is_idle(
                        &AgentMode::Specialist,
                        has_mcp_tools,
                        cycle,
                        IDLE_PROMPT,
                        false
                    ),
                    "has_mcp_tools={has_mcp_tools} cycle={cycle}"
                );
            }
        }
    }

    #[test]
    fn a_toolless_orchestrator_keeps_its_first_cycle() {
        assert!(!is_idle(
            &AgentMode::Orchestrator,
            false,
            1,
            IDLE_PROMPT,
            false
        ));
        assert!(is_idle(
            &AgentMode::Orchestrator,
            false,
            2,
            IDLE_PROMPT,
            false
        ));
    }

    #[test]
    fn an_orchestrator_with_tools_runs_every_cycle() {
        assert!(!is_idle(
            &AgentMode::Orchestrator,
            true,
            7,
            IDLE_PROMPT,
            false
        ));
    }

    #[test]
    fn a_cycle_with_work_or_a_requester_is_never_idle() {
        assert!(!is_idle(
            &AgentMode::Specialist,
            false,
            3,
            "## Incoming Messages\nhello",
            false
        ));
        assert!(!is_idle(
            &AgentMode::Specialist,
            false,
            3,
            IDLE_PROMPT,
            true
        ));
    }
}
