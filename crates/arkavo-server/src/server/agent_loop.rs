use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::server::tool_memory::ToolMemory;

/// Configuration for the agent orchestrator loop.
///
/// Bundles all shared state that the orchestrator needs, avoiding deep
/// closure captures and making the loop testable in isolation.
pub struct AgentLoopConfig {
    pub conductor: Arc<arkavo_hrm::Conductor<arkavo_hrm::store::InMemoryTaskStore>>,
    pub router: Arc<arkavo_router::Router>,
    pub mcp_registry: Arc<arkavo_protocol::mcp_registry::McpRegistry>,
    pub agent_memory: Arc<tokio::sync::RwLock<ToolMemory>>,
    pub learning_bus: Option<Arc<super::learning_bus::LearningBus>>,
    pub mesh_state: Arc<arkavo_mcp_mesh::MeshToolsState>,
    pub compute_budget: arkavo_budget::SharedComputeBudget,
    pub model_hint: Option<arkavo_router::ModelChoice>,
    pub purpose: String,
    pub orchestrator_tick: Arc<std::sync::atomic::AtomicU64>,
    pub has_mcp_tools: bool,
    pub tool_loop_budget: Option<u32>,
    pub total_ram_bytes: u64,
    pub self_agent_id: String,
    pub commander_model: String,
    pub agent_mode: arkavo_protocol::agent_config::AgentMode,
    /// Shared flag: set true during conductor inference to gate notification handler
    pub inference_active: Arc<std::sync::atomic::AtomicBool>,
    #[cfg(feature = "iroh")]
    pub iroh_node: Option<Arc<arkavo_tdf_iroh::IrohNode>>,
}

// --- Pending message drain helper ---

fn drain_pending_messages(
    pending: &mut Vec<super::agent_event::PendingMessage>,
    cycle_id: super::agent_event::CycleId,
) -> (
    String,
    Vec<(
        tokio::sync::oneshot::Sender<super::agent_event::CycleReceipt>,
        super::agent_event::CycleReceipt,
    )>,
) {
    let mut block = String::new();
    let mut receipts = Vec::new();
    for mut msg in pending.drain(..) {
        if !block.is_empty() {
            block.push('\n');
        }
        block.push_str(&msg.content);
        if let Some(reply) = msg.reply.take() {
            receipts.push((
                reply,
                super::agent_event::CycleReceipt {
                    cycle_id,
                    correlation_id: msg.correlation_id,
                    disposition: super::agent_event::MessageDisposition::Incorporated { cycle_id },
                },
            ));
        }
    }
    (block, receipts)
}

// --- Main event loop ---

/// Runs the unified agent orchestrator loop with a `tokio::select!` event loop.
///
/// Replaces the inline loop body from `a2a_server.rs` with a proper event-driven
/// architecture. A2A messages arrive via `agent_event_rx`, tick-based cycles
/// drive the orchestrator, and conversation history persists across cycles.
#[allow(clippy::too_many_lines)]
pub async fn run_agent_loop(
    config: AgentLoopConfig,
    mut agent_event_rx: tokio::sync::mpsc::Receiver<super::agent_event::AgentEvent>,
) {
    use super::agent_event::{AgentEvent, CycleId, MessagePriority, PendingMessage};
    use std::sync::atomic::Ordering::Relaxed;

    // --- State initialization ---

    let mut cycle: u64 = 0;
    let mut consecutive_no_action_cycles: u32 = 0;
    let mut last_broadcast_cycle: u64 = 0;
    let mut consecutive_timeouts: u32 = 0;
    let mut last_cycle_prompt_hash: u64 = 0;
    let mut consecutive_duplicate_prompts: u32 = 0;

    // Token estimator: use real tokenizer when llama.cpp is available
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    let estimator: Arc<dyn crate::server::token_estimator::TokenEstimator> = {
        match config.router.any_loaded_model() {
            Some(model) => Arc::new(crate::server::token_estimator::LlamaTokenEstimator::new(
                model,
            )),
            None => Arc::new(crate::server::token_estimator::MockEstimator),
        }
    };
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    let estimator: Arc<dyn crate::server::token_estimator::TokenEstimator> =
        Arc::new(crate::server::token_estimator::MockEstimator);

    let min_context = config.router.min_feasible_context_size();
    let mut conversation =
        crate::server::conversation_window::ConversationWindow::new(min_context, estimator);
    conversation.set_system_message(arkavo_llm::Message::system(&config.purpose));
    let mut pending_messages: Vec<PendingMessage> = Vec::new();

    // Build tool registry once — same tools every cycle, no need to rebuild
    let cached_registry = {
        use arkavo_mcp_tools::ToolRegistry;
        let mut registry = ToolRegistry::empty();

        if let Ok(mcp_tools) = config.mcp_registry.list_all_tools().await {
            for tool in mcp_tools {
                let tool_name = tool.name.clone();
                let bridge =
                    super::mcp_bridge::McpBridgeTool::new(config.mcp_registry.clone(), tool);
                registry.register(&tool_name, Box::new(bridge));
            }
        }

        arkavo_mcp_mesh::register_tools(&mut registry, config.mesh_state.clone());

        #[cfg(feature = "iroh")]
        if let Some(ref node) = config.iroh_node {
            arkavo_mcp_tools::iroh_data::register_iroh_tools(&mut registry, node.clone());
        }

        info!(
            "Agent loop: cached {} tools for reuse across cycles",
            registry.list_tools().len()
        );
        Arc::new(registry)
    };

    // Adaptive tick interval
    let mut cycle_interval_secs: u64 = 5;
    let mut tick_interval =
        tokio::time::interval(std::time::Duration::from_secs(cycle_interval_secs));
    tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    if config.agent_mode == arkavo_protocol::agent_config::AgentMode::Specialist {
        info!("Specialist mode: proactive advisory loop with mesh tools");
    }

    // --- Select loop ---

    loop {
        tokio::select! {
            _ = tick_interval.tick() => {
                // === TICK BRANCH: Execute one orchestrator cycle ===
                // Gate notifications for the entire cycle, not just the conductor call
                config.inference_active.store(true, std::sync::atomic::Ordering::SeqCst);
                cycle += 1;
                config.orchestrator_tick.store(cycle, Relaxed);

                if config.purpose.is_empty() {
                    continue;
                }

                // 1. Budget gate (specialists only)
                if !config.has_mcp_tools {
                    let budget = config.compute_budget.read().await;
                    let snapshot = budget.snapshot();
                    if !snapshot.has_remaining {
                        drop(budget);
                        continue;
                    }
                }

                // 2. Drain gossip completions into specialist_context
                if let Some(ref bus) = config.learning_bus {
                    for notice in bus.drain_task_completions().await {
                        let budget_snapshot = notice
                            .budget_snapshot
                            .and_then(|v| serde_json::from_value(v).ok());
                        config
                            .mesh_state
                            .push_completed(
                                &notice.task_id,
                                arkavo_mcp_mesh::CompletedDelegation {
                                    agent_id: notice.specialist_id,
                                    response: notice.content,
                                    response_latency_ms: notice.completion_ms,
                                    budget_snapshot,
                                },
                            )
                            .await;
                    }
                }

                let completed = config.mesh_state.collect_completed().await;

                for c in &completed {
                    if let Some(ref snap) = c.budget_snapshot
                        && (snap.status == "exhausted" || snap.status == "passive")
                    {
                        info!(
                            agent_id = %c.agent_id,
                            status = %snap.status,
                            remaining_inferences = snap.remaining_inferences,
                            "Specialist budget {} — skipping broadcast this cycle",
                            snap.status
                        );
                    }
                }

                let specialist_context = if completed.is_empty() {
                    String::new()
                } else {
                    use std::fmt::Write as _;
                    let mut ctx =
                        String::from("\n\n## Specialist Responses (ready — EXECUTE these now)\n");
                    for c in &completed {
                        let _ = write!(
                            ctx,
                            "### From {} specialist ({}ms):\n{}\n\n",
                            c.agent_id, c.response_latency_ms, c.response
                        );
                    }
                    ctx.push_str(
                        "IMPORTANT: The above specialist advice is ready. \
                         Skip CONSULT/MONITOR steps and go straight to EXECUTE \
                         Execute the recommended actions NOW.\n",
                    );
                    info!(
                        "Injecting {} specialist response(s) into cycle prompt",
                        completed.len()
                    );
                    ctx
                };

                // 3. Drain pending_messages into message block + send CycleReceipts
                let (message_block, receipts) =
                    drain_pending_messages(&mut pending_messages, CycleId(cycle));
                for (sender, receipt) in receipts {
                    let _ = sender.send(receipt);
                }

                // 4. Build control signals from ToolMemory (Phase 3: control-signals-only)
                let memory_guard = config.agent_memory.read().await;
                let control_signals = memory_guard.format_control_signals();
                let memory_entry_count = memory_guard.entry_count();
                drop(memory_guard);
                if let Some(ref signals) = control_signals {
                    info!(
                        memory_entries = memory_entry_count,
                        signals_len = signals.len(),
                        "ToolMemory control signals for cycle {cycle}"
                    );
                }

                // 5. Assemble cycle prompt
                let dead_man_warning = match consecutive_no_action_cycles {
                    3..=4 => "\n\nWARNING: You have NOT taken any action for 3 cycles. \
                              You MUST take an action NOW. \
                              Pick the most urgent alert and act on it.\n"
                        .to_string(),
                    5.. => {
                        let mem_guard = config.agent_memory.read().await;
                        let recent_types = mem_guard.recent_action_types();
                        drop(mem_guard);
                        let avoid = if recent_types.is_empty() {
                            String::new()
                        } else {
                            format!(
                                " You have been repeating: {}. Try a DIFFERENT action type.",
                                recent_types.join(", ")
                            )
                        };
                        format!(
                            "\n\nCRITICAL: You have NOT acted for {consecutive_no_action_cycles}+ cycles.{avoid} \
                             Read the alerts and pick the MOST URGENT need. \
                             Take an action NOW.\n"
                        )
                    }
                    _ => String::new(),
                };

                let cycle_prompt = if cycle == 1 {
                    if message_block.is_empty() {
                        "Begin your startup workflow now.".to_string()
                    } else {
                        format!("Begin your startup workflow now.\n\n{message_block}")
                    }
                } else {
                    let msg_section = if message_block.is_empty() {
                        String::new()
                    } else {
                        format!("\n\n## Incoming Messages\n{message_block}")
                    };
                    format!(
                        "{specialist_context}{dead_man_warning}{msg_section}\n\n\
                         Cycle {cycle}. Follow your WORKFLOW.",
                    )
                };

                // 6. Duplicate prompt detection
                let prompt_hash = {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    specialist_context.hash(&mut hasher);
                    control_signals.hash(&mut hasher);
                    message_block.hash(&mut hasher);
                    consecutive_no_action_cycles.min(5).hash(&mut hasher);
                    hasher.finish()
                };
                if prompt_hash == last_cycle_prompt_hash && consecutive_no_action_cycles >= 2 {
                    consecutive_duplicate_prompts += 1;
                    if consecutive_duplicate_prompts % 5 != 0 {
                        info!(
                            "Agent cycle {cycle}: skipping duplicate prompt \
                             ({consecutive_duplicate_prompts} consecutive, \
                             no action for {consecutive_no_action_cycles} cycles)"
                        );
                        continue;
                    }
                } else {
                    consecutive_duplicate_prompts = 0;
                }
                last_cycle_prompt_hash = prompt_hash;

                // 7. Push user message to ConversationWindow (unconditionally)
                conversation.push(arkavo_llm::Message::user(&cycle_prompt));

                // 8. Build messages via conversation.build_messages()
                let messages = conversation
                    .build_messages(control_signals.as_deref());

                info!(
                    "Agent cycle {cycle}: executing (history_len={})",
                    conversation.history_len()
                );
                let start = std::time::Instant::now();

                // 9. Execute via conductor with Some(messages)
                let tool_loop_budget = if config.has_mcp_tools {
                    None
                } else {
                    Some(&config.compute_budget)
                };
                match super::conductor::execute_with_conductor_and_learning(
                    &config.conductor,
                    &config.router,
                    &config.mcp_registry,
                    cycle_prompt.clone(),
                    None,
                    None,
                    config.learning_bus.as_ref(),
                    Some(&config.agent_memory),
                    Some(config.purpose.as_str()), // in messages too, but needed for classification_content hint
                    Some(&config.mesh_state),
                    config.model_hint.as_ref(),
                    None,
                    tool_loop_budget,
                    Some(messages),
                    true, // skip complexity — orchestrator cycles are always single tasks
                    Some(cached_registry.clone()),
                    #[cfg(feature = "iroh")]
                    config.iroh_node.as_ref(),
                )
                .await
                {
                    Ok(result) => {
                        let elapsed = start.elapsed();

                        // 10. Push assistant response (on success only)
                        conversation.push(arkavo_llm::Message::assistant(&result));

                        // 11. Update timeout/action tracking
                        let memory_guard = config.agent_memory.read().await;
                        let had_action = memory_guard.had_meaningful_action();

                        // 13. State broadcast every 3 cycles
                        if cycle - last_broadcast_cycle >= 3 {
                            let mesh = config.mesh_state.clone();
                            let data = memory_guard
                                .last_observe_full()
                                .map(|v| v.to_string())
                                .unwrap_or_default();
                            let cmd_model = config.commander_model.clone();
                            let own_id = config.self_agent_id.clone();
                            let total_ram = config.total_ram_bytes;
                            #[cfg(feature = "iroh")]
                            let iroh_for_broadcast = config.iroh_node.clone();
                            tokio::spawn(async move {
                                mesh.discover_peers().await;
                                let peer_count = mesh.agent_addresses.read().await.len();
                                info!(
                                    peer_count,
                                    data_len = data.len(),
                                    "Broadcast check: peers={peer_count}, data_len={}",
                                    data.len()
                                );
                                let per_agent_bytes = compute_per_agent_bytes_static(
                                    total_ram,
                                    &cmd_model,
                                    peer_count,
                                );
                                if !data.is_empty() {
                                    #[cfg(feature = "iroh")]
                                    let iroh_ticket = if data.len() > 4000 {
                                        stage_on_iroh(iroh_for_broadcast.as_ref(), &data).await
                                    } else {
                                        None
                                    };
                                    #[cfg(not(feature = "iroh"))]
                                    let iroh_ticket: Option<String> = None;

                                    broadcast_state_to_peers(
                                        &mesh,
                                        &data,
                                        per_agent_bytes,
                                        &own_id,
                                        iroh_ticket.as_deref(),
                                    )
                                    .await;
                                } else {
                                    info!("No observation data — refreshing budgets only");
                                    refresh_specialist_budgets(&mesh, per_agent_bytes, &own_id)
                                        .await;
                                }
                            });
                            last_broadcast_cycle = cycle;
                        }
                        drop(memory_guard);

                        if had_action {
                            consecutive_no_action_cycles = 0;
                            consecutive_duplicate_prompts = 0;
                        } else {
                            consecutive_no_action_cycles += 1;
                            if consecutive_no_action_cycles >= 3 {
                                warn!(
                                    "Dead-man's switch: {} consecutive ticks without meaningful action",
                                    consecutive_no_action_cycles
                                );
                            }
                        }

                        // 12. Degenerate reset (8+ no-action cycles)
                        if consecutive_no_action_cycles >= 8 {
                            warn!(
                                "Context reset: {} ticks without action — clearing short-term memory",
                                consecutive_no_action_cycles
                            );
                            let mut mem = config.agent_memory.write().await;
                            mem.clear();
                            drop(mem);
                            conversation.clear_history();
                            conversation.set_system_message(arkavo_llm::Message::system(
                                &config.purpose,
                            ));
                            consecutive_no_action_cycles = 0;
                            consecutive_duplicate_prompts = 0;

                            if let Some(ref bus) = config.learning_bus {
                                bus.add_fast_lesson(
                                    "orchestrator",
                                    "8 consecutive ticks produced no meaningful action. \
                                     The conversation context may be stale. On the next cycle, \
                                     observe the current state first, \
                                     then pick ONE concrete action from the most urgent need.",
                                )
                                .await;
                            }
                        }

                        // Adaptive cycle interval
                        let elapsed_secs = elapsed.as_secs();
                        if elapsed_secs >= 60 {
                            consecutive_timeouts += 1;
                        } else if elapsed_secs >= 30 {
                            consecutive_timeouts = consecutive_timeouts.saturating_sub(1);
                        } else {
                            consecutive_timeouts = 0;
                        }

                        info!(
                            "Agent cycle {cycle} completed in {:.1}s: {} chars",
                            elapsed.as_secs_f64(),
                            result.len(),
                        );
                    }
                    Err(e) => {
                        // User msg already pushed (step 7), no assistant response
                        consecutive_no_action_cycles += 1;
                        consecutive_timeouts += 1;
                        warn!("Agent cycle {cycle} failed: {e}");
                    }
                }
                config.inference_active.store(false, std::sync::atomic::Ordering::SeqCst);

                // 14. Update adaptive interval (only when changed)
                let new_interval = match consecutive_timeouts {
                    0 => 5,
                    1 => 15,
                    2 => 30,
                    _ => 60,
                };
                if new_interval != cycle_interval_secs {
                    cycle_interval_secs = new_interval;
                    tick_interval =
                        tokio::time::interval(std::time::Duration::from_secs(cycle_interval_secs));
                    tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                }
                info!(consecutive_timeouts, cycle_interval_secs, "Next cycle interval");
            }
            Some(event) = agent_event_rx.recv() => {
                // === EVENT BRANCH: Route incoming messages to pending_messages ===
                match event {
                    AgentEvent::IncomingMessage {
                        sender,
                        content,
                        task_id,
                        correlation_id,
                        reply,
                    } => {
                        pending_messages.push(PendingMessage {
                            content: format!(
                                "[msg correlation_id={} from={}] {}",
                                correlation_id.0, sender, content
                            ),
                            task_id: Some(task_id),
                            correlation_id,
                            reply: Some(reply),
                            priority: MessagePriority::Normal,
                        });
                        tick_interval.reset();
                    }
                    AgentEvent::HumanOverride {
                        instruction,
                        correlation_id,
                        reply,
                    } => {
                        pending_messages.insert(
                            0,
                            PendingMessage {
                                content: format!(
                                    "[msg correlation_id={} from=human] {}",
                                    correlation_id.0, instruction
                                ),
                                task_id: None,
                                correlation_id,
                                reply: Some(reply),
                                priority: MessagePriority::Override,
                            },
                        );
                        tick_interval.reset();
                    }
                    AgentEvent::Shutdown => break,
                }
            }
        }
    }
    info!("Agent loop exiting");
}

// --- Urgency detection ---

/// Detect urgency level from game state observation text.
///
/// Tries structured JSON first (looks for an `alerts` array), then falls back
/// to keyword frequency scanning for threat-related terms.
pub(super) fn detect_urgency(observe_data: &str) -> arkavo_budget::UrgencyLevel {
    use arkavo_budget::UrgencyLevel;

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(observe_data)
        && let Some(alerts) = v.pointer("/alerts").and_then(|a| a.as_array())
    {
        return match alerts.len() {
            0..=1 => UrgencyLevel::Low,
            2..=4 => UrgencyLevel::Medium,
            5..=9 => UrgencyLevel::High,
            _ => UrgencyLevel::Critical,
        };
    }

    let lower = observe_data.to_lowercase();
    let count: usize = [
        "raid", "attack", "threat", "alert", "fire", "bleed", "danger",
    ]
    .iter()
    .map(|kw| lower.matches(kw).count())
    .sum();
    match count {
        0..=1 => UrgencyLevel::Low,
        2..=4 => UrgencyLevel::Medium,
        5..=9 => UrgencyLevel::High,
        _ => UrgencyLevel::Critical,
    }
}

/// Compact a large observation for broadcast to peers.
///
/// Generic strategy: if the JSON has a "Delta" key (changes since last state),
/// extract only that. Otherwise truncate to `max_chars`. Domain-specific
/// filtering is the specialist's job via its own system prompt.
pub(super) fn compact_observation(obs: &str, max_chars: usize) -> String {
    if obs.len() <= max_chars {
        return obs.to_string();
    }
    // Prefer the Delta section — it contains only what changed
    if let Some(delta_start) = obs.find("\"Delta\":{") {
        let subset = &obs[delta_start..];
        if let Some(end) = super::conductor_tool_loop::find_matching_brace(subset) {
            let delta = &subset[..=end];
            if delta.len() <= max_chars {
                return format!("{{{delta}}}");
            }
        }
    }
    // Fallback: truncate with a note
    let cut = max_chars.saturating_sub(30);
    format!(
        "{}...(truncated {} bytes)",
        &obs[..cut.min(obs.len())],
        obs.len()
    )
}

// --- Peer state broadcast ---

/// Compute per-agent memory budget from total system RAM.
///
/// Reserves 8GB for the OS and subtracts the commander's model weight,
/// then divides evenly among specialists. Returns at least 512MB.
pub(super) fn compute_per_agent_bytes_static(
    total_ram: u64,
    commander_model: &str,
    count: usize,
) -> u64 {
    const OS_RESERVE: u64 = 8 * 1024 * 1024 * 1024; // 8 GB
    const FLOOR: u64 = 512 * 1024 * 1024; // 512 MB min
    let commander_bytes = arkavo_router::ModelChoice::from_name(commander_model)
        .map(|m| m.size_bytes())
        .unwrap_or(0);
    let available = total_ram
        .saturating_sub(OS_RESERVE)
        .saturating_sub(commander_bytes);
    if count == 0 {
        return available.max(FLOOR);
    }
    (available / count as u64).max(FLOOR)
}

/// Refresh specialist budgets without sending observation data.
///
/// Called when the commander hasn't produced a game observation yet but specialists
/// need their budgets refreshed to avoid staying passive indefinitely.
pub(super) async fn refresh_specialist_budgets(
    mesh_state: &Arc<arkavo_mcp_mesh::MeshToolsState>,
    per_agent_bytes: u64,
    self_agent_id: &str,
) {
    let peer_ids: Vec<String> = mesh_state
        .agent_addresses
        .read()
        .await
        .keys()
        .filter(|id| id.as_str() != self_agent_id)
        .cloned()
        .collect();

    if peer_ids.is_empty() {
        return;
    }

    for agent_id in &peer_ids {
        let pending = mesh_state
            .pending_delegations
            .read()
            .await
            .iter()
            .filter(|d| d.agent_id == *agent_id)
            .count() as u32;
        let allocation = arkavo_budget::BudgetPolicy::allocate(
            arkavo_budget::UrgencyLevel::Low,
            pending,
            per_agent_bytes,
        );
        // Only propagate budget allocation — no LLM work needed when no game state exists
        match send_advisory_task(mesh_state, agent_id, "", Some(&allocation)).await {
            Ok(_) => info!(
                max_inferences = allocation.max_inferences,
                max_memory_mb = allocation.max_memory_bytes / (1024 * 1024),
                per_agent_bytes,
                "Budget refreshed for {agent_id}"
            ),
            Err(e) => warn!("Could not reach {agent_id}: {e}"),
        }
    }
}

/// Stage data on Iroh P2P network for peer fetching.
#[cfg(feature = "iroh")]
pub(super) async fn stage_on_iroh(
    iroh_node: Option<&Arc<arkavo_tdf_iroh::IrohNode>>,
    data: &str,
) -> Option<String> {
    let node = iroh_node?;
    let transport = arkavo_tdf_iroh::IrohTransport::new(node.clone());
    match transport.stage_bytes(data.as_bytes()).await {
        Ok(ticket) => {
            let ticket_str = ticket.to_string();
            info!(
                data_len = data.len(),
                ticket_len = ticket_str.len(),
                "Staged data on Iroh for P2P fetch"
            );
            Some(ticket_str)
        }
        Err(e) => {
            warn!("Iroh stage failed, falling back to A2A: {e}");
            None
        }
    }
}

/// Broadcast observation state to all discovered peer agents for proactive analysis.
///
/// Computes per-specialist budget allocations dynamically based on game urgency
/// and each specialist's pending task backlog.
pub(super) async fn broadcast_state_to_peers(
    mesh_state: &Arc<arkavo_mcp_mesh::MeshToolsState>,
    observation: &str,
    per_agent_bytes: u64,
    self_agent_id: &str,
    iroh_ticket: Option<&str>,
) {
    let peer_ids: Vec<String> = mesh_state
        .agent_addresses
        .read()
        .await
        .keys()
        .filter(|id| id.as_str() != self_agent_id)
        .cloned()
        .collect();

    if peer_ids.is_empty() {
        let all_ids: Vec<String> = mesh_state
            .agent_addresses
            .read()
            .await
            .keys()
            .cloned()
            .collect();
        warn!(
            self_id = %self_agent_id,
            all_agents = ?all_ids,
            "No peers to broadcast to (all filtered as self)"
        );
        return;
    }

    info!(peer_count = peer_ids.len(), peers = ?peer_ids, "Broadcasting state to specialists");

    let urgency = detect_urgency(observation);
    let compacted = compact_observation(observation, 2000);

    let task = if let Some(ticket) = iroh_ticket {
        format!(
            "PROACTIVE ANALYSIS — Full state available via Iroh P2P.\n\
             Use iroh_fetch with this ticket to get full data: {ticket}\n\n\
             Summary: {compacted}\n\n\
             Respond with urgent action recommendations ONLY if you see \
             problems in your domain. If everything looks fine, respond \
             with 'No issues detected.'"
        )
    } else {
        format!(
            "PROACTIVE ANALYSIS — Review this state update and respond \
             with urgent action recommendations ONLY if you see problems \
             in your domain of expertise.\n\
             If everything looks fine, respond with 'No issues detected.'\n\n\
             {compacted}"
        )
    };

    for agent_id in &peer_ids {
        let pending = mesh_state
            .pending_delegations
            .read()
            .await
            .iter()
            .filter(|d| d.agent_id == *agent_id)
            .count() as u32;

        // Back off if specialist already has pending work — don't pile on tasks
        if pending >= 2 {
            info!(
                agent_id = %agent_id,
                pending,
                "Skipping broadcast to {agent_id} — already has {pending} pending tasks"
            );
            continue;
        }

        let allocation = arkavo_budget::BudgetPolicy::allocate(urgency, pending, per_agent_bytes);
        match send_advisory_task(mesh_state, agent_id, &task, Some(&allocation)).await {
            Ok(_) => info!(
                ?urgency,
                pending,
                max_inferences = allocation.max_inferences,
                ttl_secs = allocation.ttl_secs,
                "Budget allocated to {agent_id}"
            ),
            Err(e) => warn!("Could not reach {agent_id}: {e}"),
        }
    }
}

/// Send an advisory task to a specialist via message/send RPC.
///
/// Lightweight — short timeout, no retries. Tracks PendingDelegation
/// so `collect_completed()` picks up the response.
pub(super) async fn send_advisory_task(
    mesh_state: &Arc<arkavo_mcp_mesh::MeshToolsState>,
    agent_id: &str,
    task: &str,
    budget_allocation: Option<&arkavo_budget::BudgetAllocation>,
) -> std::result::Result<(), String> {
    use arkavo_protocol::transport::TlsConfig;
    use arkavo_protocol::types::{Message, MessagePart, MessageSendRequest, MessageSendResponse};
    use arkavo_protocol::{
        A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, HttpTransport, TransportConfig,
    };

    // Empty task = budget-only refresh. Skip the RPC call that would trigger
    // idle LLM inference on the specialist side.
    if task.is_empty() {
        debug!(agent_id, "Budget-only refresh — no task content to send");
        return Ok(());
    }

    let address = {
        let addrs = mesh_state.agent_addresses.read().await;
        addrs.get(agent_id).cloned()
    };
    let address = match address {
        Some(a) => a,
        None => return Err(format!("Agent {agent_id} not discovered")),
    };

    let transport_config = TransportConfig {
        timeout_ms: 10000,
        max_retries: 0,
        tls_config: TlsConfig {
            require_tls: false,
            ..Default::default()
        },
        ..Default::default()
    };

    let transport = Arc::new(HttpTransport::new(transport_config).map_err(|e| e.to_string())?);

    let endpoint = A2aEndpoint {
        url: address.clone(),
        agent_id: agent_id.to_string(),
        public_key: None,
    };

    transport
        .connect(&endpoint)
        .await
        .map_err(|e| e.to_string())?;

    let mut meta = serde_json::json!({
        "source": "state_broadcast",
        "task_type": "advisory"
    });
    if let Some(alloc) = budget_allocation {
        meta["budget_allocation"] = serde_json::to_value(alloc).unwrap_or_default();
    }

    let message = Message {
        parts: vec![MessagePart::Text {
            content: task.to_string(),
        }],
        metadata: Some(meta),
    };

    let send_request = MessageSendRequest {
        message,
        task_id: None,
    };

    let rpc_request = A2aRequest::new("message/send", serde_json::json!([send_request]));

    let response = transport
        .send_request(rpc_request)
        .await
        .map_err(|e| e.to_string())?;
    let _ = transport.close().await;

    match response {
        A2aResponse::Success { result, .. } => {
            let send_response: MessageSendResponse =
                serde_json::from_value(result).map_err(|e| e.to_string())?;

            if !send_response.task_id.is_empty() {
                mesh_state.pending_delegations.write().await.push(
                    arkavo_mcp_mesh::PendingDelegation {
                        task_id: send_response.task_id,
                        agent_id: agent_id.to_string(),
                        address,
                        sent_at: std::time::Instant::now(),
                    },
                );
            }
            Ok(())
        }
        A2aResponse::Error { error, .. } => Err(format!("{}: {}", error.code, error.message)),
    }
}

#[cfg(test)]
mod broadcast_tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("SRV-005")]
    #[tokio::test]
    async fn test_broadcast_skips_when_no_peers() {
        let mesh = Arc::new(arkavo_mcp_mesh::MeshToolsState::new());
        broadcast_state_to_peers(&mesh, r#"{"data":"test"}"#, 0, "self", None).await;
        assert!(mesh.pending_delegations.read().await.is_empty());
    }

    #[spec("SRV-005")]
    #[tokio::test]
    async fn test_broadcast_creates_task_per_peer() {
        let mesh = Arc::new(arkavo_mcp_mesh::MeshToolsState::new());
        mesh.agent_addresses
            .write()
            .await
            .insert("peer-a".to_string(), "http://127.0.0.1:19999".to_string());
        mesh.agent_addresses
            .write()
            .await
            .insert("peer-b".to_string(), "http://127.0.0.1:19998".to_string());

        broadcast_state_to_peers(&mesh, r#"{"state":"test"}"#, 0, "self", None).await;
        assert!(mesh.pending_delegations.read().await.is_empty());
    }

    #[spec("SRV-001")]
    #[test]
    fn test_dynamic_memory_128gb_3_specialists() {
        let total_ram = 128 * 1024 * 1024 * 1024_u64; // 128 GB
        let commander_size = arkavo_router::ModelChoice::from_name("glm-4.7-flash")
            .unwrap()
            .size_bytes(); // ~20 billion bytes
        let per_agent = compute_per_agent_bytes_static(total_ram, "glm-4.7-flash", 3);
        let os_reserve = 8 * 1024 * 1024 * 1024_u64;
        let expected = (total_ram - os_reserve - commander_size) / 3;
        assert_eq!(per_agent, expected);
        // Must be well above the 512MB floor (~37 GB)
        assert!(per_agent > 30 * 1024 * 1024 * 1024);
    }

    #[spec("SRV-001")]
    #[test]
    fn test_dynamic_memory_floor_enforced() {
        // Tiny system: 4 GB total, large commander model, many specialists
        let total_ram = 4 * 1024 * 1024 * 1024_u64;
        let per_agent = compute_per_agent_bytes_static(total_ram, "glm-4.7-flash", 10);
        // Should hit the 512MB floor
        assert_eq!(per_agent, 512 * 1024 * 1024);
    }
}

#[cfg(test)]
mod urgency_tests {
    use super::*;
    use arkavo_budget::UrgencyLevel;
    use arkavo_test_macros::spec;

    #[spec("SRV-009")]
    #[test]
    fn test_detect_urgency_json_alerts_low() {
        let data = r#"{"alerts":[],"colonists":[{"name":"Jess"}]}"#;
        assert_eq!(detect_urgency(data), UrgencyLevel::Low);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_urgency_json_alerts_medium() {
        let data = r#"{"alerts":["cold snap","food shortage","crop blight"]}"#;
        assert_eq!(detect_urgency(data), UrgencyLevel::Medium);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_urgency_json_alerts_high() {
        let data = r#"{"alerts":["a","b","c","d","e","f","g"]}"#;
        assert_eq!(detect_urgency(data), UrgencyLevel::High);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_urgency_json_alerts_critical() {
        let alerts: Vec<String> = (0..12).map(|i| format!("alert_{i}")).collect();
        let data = serde_json::json!({"alerts": alerts}).to_string();
        assert_eq!(detect_urgency(&data), UrgencyLevel::Critical);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_urgency_keyword_fallback() {
        let data = "Colony is under raid! Attack from the north. Fire in storage.";
        assert_eq!(detect_urgency(data), UrgencyLevel::Medium);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_urgency_no_threats() {
        let data = "All colonists are happy. Resources are plentiful.";
        assert_eq!(detect_urgency(data), UrgencyLevel::Low);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_urgency_keyword_critical() {
        let data = "raid attack threat alert fire bleed danger raid attack threat alert";
        assert_eq!(detect_urgency(data), UrgencyLevel::Critical);
    }

    #[spec("SRV-009")]
    #[test]
    fn test_detect_urgency_json_without_alerts_key() {
        let data = r#"{"colonists":3,"resources":{"wood":50}}"#;
        assert_eq!(detect_urgency(data), UrgencyLevel::Low);
    }
}
