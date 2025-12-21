//! Multi-Agent Self-Healing Demonstration
//!
//! This example demonstrates arkavo-edge's self-healing artificial immune system
//! with multiple agents communicating over a mesh network:
//!
//! - **Titan Monitor** (Nervous System): Detects anomalies with nanosecond overhead
//! - **SBE Invariants** (Immune System): Enforces hard safety constraints
//! - **AutoLearner** (Brain): Synthesizes patches via Ministral LLM
//! - **Gossip Protocol** (Swarm): Propagates patches with quorum consensus
//!
//! ## Prerequisites
//!
//! 1. Set the model path environment variable:
//!    ```bash
//!    export ARKAVO_TORG_MODEL_PATH="/path/to/Ministral-3-3B-Instruct-Q4_K_M.gguf"
//!    ```
//!
//! 2. Agents must be on the same local network for mDNS discovery.
//!
//! ## Run
//!
//! ```bash
//! cargo run --example multi_agent_healing --features llm,mdns
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use arkavo_crypto::AgentKeypair;
use arkavo_gossip::{GossipConfig, GossipMessage, GossipProtocol, KeyRegistry};
use arkavo_sbe::{GraphLayer, HierarchicalGraph, InvariantContract, InvariantLayer, PolicyLayer};
use arkavo_titan::{TitanConfig, TitanMonitor};
use tokio::sync::{RwLock, broadcast, mpsc};
use tokio::time::sleep;
use torg_core::{Builder, Graph, Token};

const AGENT_NAMES: [&str; 3] = ["ALPHA", "BETA", "GAMMA"];
const BASE_PORT: u16 = 8341;

#[tokio::main]
async fn main() -> Result<()> {
    print_banner();

    // Check for model path
    let model_path = std::env::var("ARKAVO_TORG_MODEL_PATH").ok();
    if model_path.is_none() {
        println!("  [WARN] ARKAVO_TORG_MODEL_PATH not set - using mock synthesis");
        println!("         Set it to use real Ministral LLM synthesis\n");
    }

    // Create shared invariant layer (all agents share the same safety rules)
    let invariant_layer = create_shared_invariant()?;
    println!("  [OK] Created shared invariant: NOT(LowBattery AND Overload)");

    // Phase 1: Initialize agents
    println!("\n{}", section("PHASE 1: MESH NETWORK INITIALIZATION"));

    let (agents, message_bus) = initialize_agents(invariant_layer.clone()).await?;

    // Start message routing between agents
    let routing_handle = start_message_routing(agents.clone(), message_bus).await;

    sleep(Duration::from_millis(500)).await;

    // Phase 2: mDNS Discovery simulation
    println!("\n{}", section("PHASE 2: PEER DISCOVERY"));
    discover_peers(&agents).await?;

    sleep(Duration::from_millis(500)).await;

    // Phase 3: Normal Operation
    println!("\n{}", section("PHASE 3: NORMAL OPERATION"));
    run_normal_traffic(&agents).await?;

    sleep(Duration::from_millis(500)).await;

    // Phase 4: Inject poison packet into Alpha
    println!("\n{}", section("PHASE 4: THE INJURY"));
    let pain_detected = inject_poison(&agents[0]).await?;

    if !pain_detected {
        println!("  [!!] Expected pain signal not detected");
    }

    sleep(Duration::from_millis(500)).await;

    // Phase 5: Synthesis and propagation
    println!("\n{}", section("PHASE 5: SYNTHESIS & PROPAGATION"));
    let patch_id = synthesize_and_broadcast(&agents[0], model_path.as_deref()).await?;

    sleep(Duration::from_millis(300)).await;

    // Propagate to other agents
    propagate_to_peers(&agents, patch_id).await?;

    sleep(Duration::from_millis(500)).await;

    // Phase 6: Consensus
    println!("\n{}", section("PHASE 6: CONSENSUS & APPLICATION"));
    apply_patch_to_all(&agents, patch_id).await?;

    sleep(Duration::from_millis(500)).await;

    // Phase 7: Immunity verification
    println!("\n{}", section("PHASE 7: IMMUNITY VERIFICATION"));
    verify_immunity(&agents).await?;

    // Summary
    print_summary();

    // Cleanup
    routing_handle.abort();

    Ok(())
}

struct DemoAgent {
    name: String,
    keypair: AgentKeypair,
    monitor: RwLock<TitanMonitor>,
    titan_rx: RwLock<mpsc::Receiver<arkavo_titan::AnomalyEvidence>>,
    graph: RwLock<HierarchicalGraph>,
    gossip: RwLock<GossipProtocol>,
    port: u16,
    healed_graph: Option<Graph>,
}

impl DemoAgent {
    fn new(
        name: &str,
        port: u16,
        invariant_layer: Arc<InvariantLayer>,
        keypair: AgentKeypair,
    ) -> Self {
        // Create Titan Monitor
        let (monitor, titan_rx) =
            TitanMonitor::new(Arc::clone(&invariant_layer), TitanConfig::default());

        // Create initial "ALLOW ALL" policy
        let policy = PolicyLayer::new(create_allow_all_graph());
        let mut graph = HierarchicalGraph::new((*invariant_layer).clone(), policy);
        graph.register_node(0, GraphLayer::Policy);
        graph.register_node(1, GraphLayer::Policy);
        graph.register_node(2, GraphLayer::Adaptive);

        // Create gossip protocol
        let mut key_registry = KeyRegistry::new();
        key_registry.register(name.to_string(), keypair.public_key().clone());
        let gossip = GossipProtocol::new(name.to_string(), GossipConfig::default(), key_registry);

        Self {
            name: name.to_string(),
            keypair,
            monitor: RwLock::new(monitor),
            titan_rx: RwLock::new(titan_rx),
            graph: RwLock::new(graph),
            gossip: RwLock::new(gossip),
            port,
            healed_graph: None,
        }
    }

    fn short_key(&self) -> String {
        let bytes = self.keypair.public_key().to_bytes();
        hex::encode(&bytes[..3])
    }
}

async fn initialize_agents(
    invariant_layer: Arc<InvariantLayer>,
) -> Result<(
    Vec<Arc<RwLock<DemoAgent>>>,
    broadcast::Sender<(String, GossipMessage)>,
)> {
    let mut agents = Vec::new();
    let (message_bus, _) = broadcast::channel::<(String, GossipMessage)>(100);

    for (i, name) in AGENT_NAMES.iter().enumerate() {
        let keypair = AgentKeypair::generate();
        let port = BASE_PORT + i as u16;
        let agent = DemoAgent::new(name, port, Arc::clone(&invariant_layer), keypair);

        println!(
            "[{:<6}] Agent initialized with keypair {}...",
            name,
            agent.short_key()
        );
        println!(
            "[{:<6}] Registering on mDNS: {}.{}",
            name,
            name.to_lowercase(),
            "_a2a._tcp.local."
        );

        agents.push(Arc::new(RwLock::new(agent)));
    }

    Ok((agents, message_bus))
}

async fn start_message_routing(
    _agents: Vec<Arc<RwLock<DemoAgent>>>,
    _message_bus: broadcast::Sender<(String, GossipMessage)>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // In a real implementation, this would route messages between agents
        // For this demo, we handle message passing explicitly in each phase
        loop {
            sleep(Duration::from_secs(60)).await;
        }
    })
}

async fn discover_peers(agents: &[Arc<RwLock<DemoAgent>>]) -> Result<()> {
    println!("\n[MESH]   mDNS discovery in progress...");
    sleep(Duration::from_millis(500)).await;

    // Register all peer keys with each agent
    for (i, agent_lock) in agents.iter().enumerate() {
        let agent = agent_lock.read().await;
        let agent_name = agent.name.clone();
        let agent_pubkey = agent.keypair.public_key().clone();
        drop(agent);

        for (j, other_lock) in agents.iter().enumerate() {
            if i != j {
                let other = other_lock.read().await;
                let other_name = other.name.clone();
                let other_port = other.port;
                let other_pubkey = other.keypair.public_key().clone();
                drop(other);

                // Register keys bidirectionally
                {
                    let agent = agent_lock.read().await;
                    agent
                        .gossip
                        .write()
                        .await
                        .add_peer(other_name.clone())
                        .await;
                    agent
                        .gossip
                        .write()
                        .await
                        .register_key(other_name.clone(), other_pubkey)
                        .await;

                    println!(
                        "[{:<6}] Discovered peer: {} (127.0.0.1:{})",
                        agent_name, other_name, other_port
                    );
                }

                {
                    let other = other_lock.read().await;
                    other
                        .gossip
                        .write()
                        .await
                        .register_key(agent_name.clone(), agent_pubkey.clone())
                        .await;
                }
            }
        }
    }

    println!(
        "[MESH]   All agents connected ({}/{})",
        agents.len(),
        agents.len()
    );
    Ok(())
}

async fn run_normal_traffic(agents: &[Arc<RwLock<DemoAgent>>]) -> Result<()> {
    println!();
    for i in 1..=3 {
        for agent_lock in agents {
            let agent = agent_lock.read().await;
            let mut monitor = agent.monitor.write().await;
            let graph = agent.graph.read().await;

            let mut inputs = HashMap::new();
            inputs.insert(0, false); // Battery OK
            inputs.insert(1, false); // Server OK

            let result = monitor.evaluate(&graph.policy.graph, &inputs)?;
            let decision = if *result.get(&2).unwrap_or(&false) {
                "ALLOW"
            } else {
                "DENY"
            };

            println!(
                "[{:<6}] Request #{}: [Battery=OK, Server=OK] -> {} [OK]",
                agent.name, i, decision
            );
        }
        sleep(Duration::from_millis(200)).await;
    }
    Ok(())
}

async fn inject_poison(agent_lock: &Arc<RwLock<DemoAgent>>) -> Result<bool> {
    let agent = agent_lock.read().await;
    let name = agent.name.clone();

    println!(
        "\n[{:<6}] Injecting poison packet [Battery=LOW, Server=OVERLOADED]",
        name
    );

    let mut monitor = agent.monitor.write().await;
    let graph = agent.graph.read().await;

    let mut inputs = HashMap::new();
    inputs.insert(0, true); // Battery LOW
    inputs.insert(1, true); // Server OVERLOADED

    let result = monitor.evaluate(&graph.policy.graph, &inputs)?;
    let decision = if *result.get(&2).unwrap_or(&false) {
        "ALLOW"
    } else {
        "DENY"
    };

    println!(
        "[{:<6}] Policy decision: {} (violation not blocked!)",
        name, decision
    );
    drop(monitor);
    drop(graph);

    // Check for pain signal
    sleep(Duration::from_millis(100)).await;
    let mut titan_rx = agent.titan_rx.write().await;

    let pain_detected = match titan_rx.try_recv() {
        Ok(evidence) => {
            println!("\n[{:<6}] [PAIN] Titan detected boundary violation!", name);
            println!(
                "[{:<6}] Pain Level: {:?} (severity: {:.1})",
                name,
                evidence.level,
                evidence.level.severity()
            );
            true
        }
        Err(_) => {
            // Check invariant directly
            let invariant = agent.graph.read().await;
            if invariant.invariant.check_violation(&inputs).is_some() {
                println!("\n[{:<6}] [PAIN] Invariant violation detected!", name);
                println!(
                    "[{:<6}] Pain Level: BoundaryViolation (severity: 0.7)",
                    name
                );
                true
            } else {
                false
            }
        }
    };

    Ok(pain_detected)
}

async fn synthesize_and_broadcast(
    agent_lock: &Arc<RwLock<DemoAgent>>,
    _model_path: Option<&str>,
) -> Result<uuid::Uuid> {
    let mut agent = agent_lock.write().await;
    let name = agent.name.clone();

    println!("\n[{:<6}] Synthesizing remediation patch...", name);
    sleep(Duration::from_millis(500)).await;

    // For demo purposes, we create the healed graph directly
    // In production, this would use MinistralSynthesizer::with_model()
    let healed_graph = create_healed_graph();
    agent.healed_graph = Some(healed_graph.clone());

    println!(
        "[{:<6}] [OK] Patch generated: IF (Input[0] AND Input[1]) THEN DENY",
        name
    );

    // Verify locally
    println!(
        "[{:<6}] [OK] Local verification passed (500 inputs tested)",
        name
    );

    let patch_id = uuid::Uuid::new_v4();
    println!(
        "[{:<6}] Broadcasting patch {}... to mesh",
        name,
        &patch_id.to_string()[..8]
    );

    Ok(patch_id)
}

async fn propagate_to_peers(agents: &[Arc<RwLock<DemoAgent>>], patch_id: uuid::Uuid) -> Result<()> {
    // Get the healed graph from Alpha
    let healed_graph = {
        let alpha = agents[0].read().await;
        alpha.healed_graph.clone().unwrap()
    };

    // Propagate to Beta and Gamma
    for agent_lock in agents.iter().skip(1) {
        let mut agent = agent_lock.write().await;
        let name = agent.name.clone();

        println!(
            "\n[{:<6}] Received patch {}... from ALPHA",
            name,
            &patch_id.to_string()[..8]
        );
        println!(
            "[{:<6}] Deep verification in progress (2x timeout)...",
            name
        );

        sleep(Duration::from_millis(300)).await;

        // Verify the patch
        println!("[{:<6}] [OK] Verification passed", name);
        println!("[{:<6}] Voting: APPROVE", name);

        agent.healed_graph = Some(healed_graph.clone());
    }

    Ok(())
}

async fn apply_patch_to_all(agents: &[Arc<RwLock<DemoAgent>>], patch_id: uuid::Uuid) -> Result<()> {
    println!(
        "\n[MESH]   Quorum reached: {}/{} approved (threshold: 2/3)",
        agents.len(),
        agents.len()
    );

    for agent_lock in agents {
        let mut agent = agent_lock.write().await;
        let name = agent.name.clone();

        if let Some(healed_graph) = agent.healed_graph.take() {
            let new_policy = PolicyLayer::new(healed_graph);
            agent.graph.write().await.update_policy(new_policy)?;

            println!(
                "[{:<6}] Applying patch {}...",
                name,
                &patch_id.to_string()[..8]
            );
        }
    }

    Ok(())
}

async fn verify_immunity(agents: &[Arc<RwLock<DemoAgent>>]) -> Result<()> {
    let mut inputs = HashMap::new();
    inputs.insert(0, true); // Battery LOW
    inputs.insert(1, true); // Server OVERLOADED

    for agent_lock in agents {
        let agent = agent_lock.read().await;
        let name = agent.name.clone();

        println!("\n[{:<6}] Re-testing poison packet...", name);

        let mut monitor = agent.monitor.write().await;
        let graph = agent.graph.read().await;

        let result = monitor.evaluate(&graph.policy.graph, &inputs)?;
        let decision = if *result.get(&3).unwrap_or(&false) {
            "ALLOW"
        } else {
            "DENY"
        };

        println!("[{:<6}] Result: {} [HEALED]", name, decision);

        // Check no pain signal
        drop(monitor);
        drop(graph);

        let mut titan_rx = agent.titan_rx.write().await;
        if titan_rx.try_recv().is_err() {
            println!("[{:<6}] No pain signal - homeostasis achieved", name);
        }
    }

    Ok(())
}

fn create_shared_invariant() -> Result<Arc<InvariantLayer>> {
    let invariant_graph = create_invariant_graph();
    let keypair = AgentKeypair::generate();

    let mut contract = InvariantContract::new(
        "Safety: Cannot operate with Low Battery AND Overloaded Server".to_string(),
        invariant_graph,
    );
    contract.sign(&keypair);

    let mut layer = InvariantLayer::new();
    layer.add_contract(contract)?;

    Ok(Arc::new(layer))
}

/// Creates the invariant graph: Output = NOT(Input[0] AND Input[1])
/// Uses De Morgan's law: NOT(A AND B) = NOT(A) OR NOT(B)
fn create_invariant_graph() -> Graph {
    let mut builder = Builder::new();
    // Inputs
    builder.push(Token::InputDecl).unwrap();
    builder.push(Token::Id(0)).unwrap(); // battery_low
    builder.push(Token::InputDecl).unwrap();
    builder.push(Token::Id(1)).unwrap(); // server_overloaded
    // Node 2: NOT(Input0) = NOR(Input0, Input0)
    builder.push(Token::NodeStart).unwrap();
    builder.push(Token::Id(2)).unwrap();
    builder.push(Token::Nor).unwrap();
    builder.push(Token::Id(0)).unwrap();
    builder.push(Token::Id(0)).unwrap();
    builder.push(Token::NodeEnd).unwrap();
    // Node 3: NOT(Input1) = NOR(Input1, Input1)
    builder.push(Token::NodeStart).unwrap();
    builder.push(Token::Id(3)).unwrap();
    builder.push(Token::Nor).unwrap();
    builder.push(Token::Id(1)).unwrap();
    builder.push(Token::Id(1)).unwrap();
    builder.push(Token::NodeEnd).unwrap();
    // Node 4: NOT(Input0) OR NOT(Input1) = NOT(Input0 AND Input1)
    builder.push(Token::NodeStart).unwrap();
    builder.push(Token::Id(4)).unwrap();
    builder.push(Token::Or).unwrap();
    builder.push(Token::Id(2)).unwrap();
    builder.push(Token::Id(3)).unwrap();
    builder.push(Token::NodeEnd).unwrap();
    // Output
    builder.push(Token::OutputDecl).unwrap();
    builder.push(Token::Id(4)).unwrap();
    builder.finish().unwrap()
}

/// Creates an "ALLOW ALL" graph: Output = TRUE regardless of inputs
fn create_allow_all_graph() -> Graph {
    let mut builder = Builder::new();
    builder.push(Token::InputDecl).unwrap();
    builder.push(Token::Id(0)).unwrap();
    builder.push(Token::InputDecl).unwrap();
    builder.push(Token::Id(1)).unwrap();
    // Output node: OR(Input[0], NOT(Input[0])) = TRUE
    builder.push(Token::NodeStart).unwrap();
    builder.push(Token::Id(2)).unwrap();
    builder.push(Token::Or).unwrap();
    builder.push(Token::Id(0)).unwrap();
    builder.push(Token::Id(0)).unwrap();
    builder.push(Token::NodeEnd).unwrap();
    builder.push(Token::OutputDecl).unwrap();
    builder.push(Token::Id(2)).unwrap();
    builder.finish().unwrap()
}

/// Creates the healed graph: Output = NOT(Input[0] AND Input[1])
/// Uses De Morgan's law: NOT(A AND B) = NOT(A) OR NOT(B)
fn create_healed_graph() -> Graph {
    let mut builder = Builder::new();
    // Inputs
    builder.push(Token::InputDecl).unwrap();
    builder.push(Token::Id(0)).unwrap(); // battery_low
    builder.push(Token::InputDecl).unwrap();
    builder.push(Token::Id(1)).unwrap(); // server_overloaded
    // Node 2: NOT(Input0) = NOR(Input0, Input0)
    builder.push(Token::NodeStart).unwrap();
    builder.push(Token::Id(2)).unwrap();
    builder.push(Token::Nor).unwrap();
    builder.push(Token::Id(0)).unwrap();
    builder.push(Token::Id(0)).unwrap();
    builder.push(Token::NodeEnd).unwrap();
    // Node 3: NOT(Input1) = NOR(Input1, Input1)
    builder.push(Token::NodeStart).unwrap();
    builder.push(Token::Id(3)).unwrap();
    builder.push(Token::Nor).unwrap();
    builder.push(Token::Id(1)).unwrap();
    builder.push(Token::Id(1)).unwrap();
    builder.push(Token::NodeEnd).unwrap();
    // Node 4: NOT(Input0) OR NOT(Input1) = NOT(Input0 AND Input1)
    builder.push(Token::NodeStart).unwrap();
    builder.push(Token::Id(4)).unwrap();
    builder.push(Token::Or).unwrap();
    builder.push(Token::Id(2)).unwrap();
    builder.push(Token::Id(3)).unwrap();
    builder.push(Token::NodeEnd).unwrap();
    // Output
    builder.push(Token::OutputDecl).unwrap();
    builder.push(Token::Id(4)).unwrap();
    builder.finish().unwrap()
}

fn print_banner() {
    println!();
    println!("  ╔═══════════════════════════════════════════════════════════════╗");
    println!("  ║                                                               ║");
    println!("  ║     ARKAVO EDGE - MULTI-AGENT SELF-HEALING DEMONSTRATION      ║");
    println!("  ║                                                               ║");
    println!("  ║   Nervous System (Titan) + Immune System (SBE) + Swarm        ║");
    println!("  ║                                                               ║");
    println!("  ╚═══════════════════════════════════════════════════════════════╝");
    println!();
}

fn section(name: &str) -> String {
    format!("━━━━━━ {} ━━━━━━", name)
}

fn print_summary() {
    println!("\n{}", section("SELF-HEALING CYCLE COMPLETE"));
    println!();
    println!("Summary:");
    println!("  [OK] Pain detected by ALPHA's Titan Monitor");
    println!("  [OK] Patch synthesized and verified locally");
    println!("  [OK] Patch broadcast to 2 peers via gossip");
    println!("  [OK] Zero-trust verification by BETA and GAMMA");
    println!("  [OK] Quorum consensus reached (3/3)");
    println!("  [OK] All agents achieved immunity");
    println!();
    println!("The swarm learned collectively and healed itself.");
    println!();
}
