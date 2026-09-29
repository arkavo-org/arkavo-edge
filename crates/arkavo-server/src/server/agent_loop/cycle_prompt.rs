//! What one agent cycle is asked to do, and whether it should run at all.
//!
//! The loop serves two kinds of agent with one tick. An orchestrator acts on
//! its own: an idle tick still runs, and a run of ticks without action earns a
//! push to act. A specialist is passive: it spends inference only on work
//! someone sent it, and answering in text is that work done, not inaction.
//! Keeping those rules here lets them be tested without a model.

use arkavo_protocol::agent_config::AgentMode;

/// Prompt of a cycle that has nothing new to work on.
pub(super) const IDLE_PROMPT: &str = "Continue.";

/// Opening words of the state broadcast a commander sends its specialists.
///
/// The broadcast arrives as an ordinary `message/send`, so its text is the
/// only thing that tells the receiving loop it is advisory rather than a task
/// someone delegated. Sender and receiver share this constant so the two
/// cannot drift apart.
pub(super) const PROACTIVE_ANALYSIS_MARKER: &str = "PROACTIVE ANALYSIS";

/// Keeps an advisor's unsolicited analysis short enough to act on.
const ADVISORY_BREVITY: &str =
    "Respond in under 200 words. No reasoning preamble. Give actionable advice only.";

/// True when `content` is a commander's state broadcast.
pub(super) fn is_proactive_advisory(content: &str) -> bool {
    content.trim_start().starts_with(PROACTIVE_ANALYSIS_MARKER)
}

/// Everything that can put work in front of a cycle.
pub(super) struct CycleInputs<'a> {
    pub specialist_context: &'a str,
    pub dead_man_warning: &'a str,
    pub message_block: &'a str,
    pub has_mcp_tools: bool,
    /// A requester delegated a task to this cycle and is waiting on its
    /// result, as opposed to a broadcast the agent may answer in a line.
    pub serving_delegated_task: bool,
}

/// Build the user prompt for a cycle.
///
/// The brevity rule is for advice nobody asked for. A delegated task is
/// governed by the role's own instructions: capping it at 200 words overrode
/// those instructions and truncated the deliverable the requester asked for.
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
    if !inputs.has_mcp_tools && !inputs.serving_delegated_task {
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

/// True when a run of cycles without a tool action means the agent is stuck.
///
/// That holds for an orchestrator, whose job is to act. A specialist that
/// answered in text did its job, and one with nothing delegated has nothing
/// to act on: pushing either to "take an action NOW" starts inference nobody
/// asked for and steers a requester's task toward an unwanted tool call.
pub(super) fn dead_man_switch_applies(mode: &AgentMode) -> bool {
    *mode != AgentMode::Specialist
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(message_block: &str, serving_delegated_task: bool) -> CycleInputs<'_> {
        CycleInputs {
            specialist_context: "",
            dead_man_warning: "",
            message_block,
            has_mcp_tools: false,
            serving_delegated_task,
        }
    }

    #[test]
    fn nothing_to_work_on_is_the_idle_prompt() {
        assert_eq!(assemble(&inputs("", false)), IDLE_PROMPT);
        assert_eq!(assemble(&inputs("", true)), IDLE_PROMPT);
    }

    #[test]
    fn incoming_messages_become_the_prompt() {
        let prompt = assemble(&inputs("[event from=game] raid", false));
        assert!(
            prompt.starts_with("## Incoming Messages\n[event from=game] raid"),
            "got {prompt}"
        );
    }

    /// Regression: the brevity rule was appended to every toolless cycle, so a
    /// task a requester delegated was capped at 200 words whatever the role's
    /// own skill instructions said.
    #[test]
    fn a_delegated_task_is_not_capped() {
        let prompt = assemble(&inputs("[msg from=requester] Write the launch brief", true));
        assert!(prompt.ends_with("Write the launch brief"), "got {prompt}");
        assert!(!prompt.contains("200 words"), "got {prompt}");
    }

    #[test]
    fn unsolicited_advice_is_still_capped() {
        let prompt = assemble(&inputs(
            "[msg from=commander] PROACTIVE ANALYSIS — review",
            false,
        ));
        assert!(prompt.ends_with(ADVISORY_BREVITY), "got {prompt}");
    }

    #[test]
    fn an_agent_with_tools_is_never_capped() {
        let mut with_tools = inputs("[event from=game] raid", false);
        with_tools.has_mcp_tools = true;
        assert!(!assemble(&with_tools).contains("200 words"));
    }

    #[test]
    fn a_state_broadcast_is_recognised_by_its_opening_words() {
        assert!(is_proactive_advisory(
            "PROACTIVE ANALYSIS — Review this state update"
        ));
        assert!(is_proactive_advisory("  PROACTIVE ANALYSIS — Full state"));
        assert!(!is_proactive_advisory(
            "Summarise the PROACTIVE ANALYSIS we ran last week"
        ));
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

    /// Regression: three text answers in a row tripped the dead-man switch,
    /// which then ran cycles of its own on a specialist with nothing to do.
    #[test]
    fn only_an_orchestrator_is_pushed_to_act() {
        assert!(dead_man_switch_applies(&AgentMode::Orchestrator));
        assert!(!dead_man_switch_applies(&AgentMode::Specialist));
    }
}
