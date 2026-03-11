//! Cognitive Switchboard Benchmark
//!
//! Demonstrates the performance difference between LLM-based routing decisions
//! (~10/sec) and compiled TOR-G graph evaluation (~1M+/sec).
//!
//! This example implements Issue #414: the "Cognitive Switchboard" pattern where
//! natural language policies are compiled once into sub-microsecond evaluable
//! TOR-G graphs.
//!
//! # Usage
//!
//! Run with simulated LLM (no model required):
//! ```bash
//! cargo run --example switchboard_benchmark
//! ```
//!
//! Run with real LLM synthesis:
//! ```bash
//! ARKAVO_BENCH_MODEL=/path/to/model.gguf cargo run --example switchboard_benchmark --features llm
//! ```

use std::collections::HashMap;
use std::time::{Duration, Instant};

use torg_core::{Builder, Graph, Token};

#[cfg(feature = "llm")]
use arkavo_autolearn::{PolicySynthesizer, SynthesizerConfig};
#[cfg(feature = "llm")]
use arkavo_ensemble::LlmSynthesizer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("--- Cognitive Switchboard Benchmark ---");
    println!("Issue #414: Neuro-Symbolic Policy Compilation\n");

    // 1. Define the routing policy as natural language
    let policy_text = r#"
        Inputs: [is_sensitive, battery_low, complexity_high, peer_available]
        Outputs: [route_local, route_delegate, route_cloud]
        Rules:
        1. If data is sensitive (input 0), NEVER go to cloud (output 2).
        2. If battery < 20% (input 1) AND peer available (input 3), delegate (output 1).
        3. Default to route local (output 0).
    "#;

    println!("Policy specification:");
    println!("{}\n", policy_text.trim());

    // 2. LLM Baseline: Real or simulated
    let use_real_llm = std::env::var("ARKAVO_BENCH_MODEL").is_ok();

    let (graph, llm_decisions_per_sec) = if use_real_llm {
        compile_with_real_llm(policy_text).await?
    } else {
        simulate_llm_baseline()
    };

    // 3. Firehose Test: 1M evaluations
    // Build inputs matching the actual graph's input IDs
    let inputs: HashMap<u16, bool> = graph
        .inputs
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, i % 2 == 0)) // Alternate true/false
        .collect();

    let iterations = 1_000_000u64;
    println!("Running {} TOR-G evaluations...", iterations);

    // Warm-up run
    for _ in 0..1000 {
        let _ = std::hint::black_box(torg_core::evaluate(&graph, &inputs));
    }

    // Timed benchmark
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = std::hint::black_box(torg_core::evaluate(&graph, &inputs));
    }
    let duration = start.elapsed();

    let torg_decisions_per_sec = iterations as f64 / duration.as_secs_f64();
    let speedup = torg_decisions_per_sec / llm_decisions_per_sec;

    // 4. Print results
    println!("\n=== Results ===");
    println!(
        "LLM Baseline:        {:>12.1} decisions/sec",
        llm_decisions_per_sec
    );
    println!(
        "TOR-G Switchboard:   {:>12.2e} decisions/sec",
        torg_decisions_per_sec
    );
    println!("Speedup Factor:      {:>12.0}x", speedup);
    println!(
        "\nTime for 1M evals:   {:>12.2} ms",
        duration.as_secs_f64() * 1000.0
    );

    // Show graph structure
    println!("\nGraph structure:");
    println!("  Inputs:  {:?}", graph.inputs);
    println!("  Outputs: {:?}", graph.outputs);
    println!("  Nodes:   {} intermediate nodes", graph.nodes.len());

    // Build inputs matching the actual graph's input IDs
    let graph_inputs: HashMap<u16, bool> = graph
        .inputs
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, i % 2 == 0)) // Alternate true/false for demo
        .collect();

    // Verify the graph produces output
    println!("\nSample evaluation:");
    match torg_core::evaluate(&graph, &graph_inputs) {
        Ok(outputs) => {
            println!("  Inputs:  {:?}", graph_inputs);
            println!("  Outputs: {:?}", outputs);
        }
        Err(e) => println!("  Evaluation error: {}", e),
    }

    Ok(())
}

/// Compile policy using real LLM synthesis
#[cfg(feature = "llm")]
async fn compile_with_real_llm(policy_text: &str) -> anyhow::Result<(Graph, f64)> {
    let model_path = std::env::var("ARKAVO_BENCH_MODEL")?;
    println!("Loading model from: {}", model_path);

    let config = SynthesizerConfig::default();
    let synthesizer = PolicySynthesizer::with_model(config, std::path::Path::new(&model_path))?;

    // Time real LLM synthesis
    println!("Compiling policy with LLM...");
    let start = Instant::now();
    let graph = synthesizer.compile_policy(policy_text).await?;
    let synthesis_time = start.elapsed();

    let decisions_per_sec = 1.0 / synthesis_time.as_secs_f64();
    println!(
        "LLM Synthesis: {:.2}s ({:.1} decisions/sec)\n",
        synthesis_time.as_secs_f64(),
        decisions_per_sec
    );

    Ok((graph, decisions_per_sec))
}

/// Placeholder for when llm feature is not enabled
#[cfg(not(feature = "llm"))]
async fn compile_with_real_llm(_policy_text: &str) -> anyhow::Result<(Graph, f64)> {
    anyhow::bail!(
        "Enable 'llm' feature to use real model synthesis:\n\
         cargo run --example switchboard_benchmark --features llm"
    );
}

/// Simulate LLM latency and return a pre-built demo graph
fn simulate_llm_baseline() -> (Graph, f64) {
    println!("Simulating LLM baseline (10 calls @ 100ms each)...");

    // Simulate 10 LLM inference calls
    for i in 0..10 {
        print!(".");
        if (i + 1) % 5 == 0 {
            print!(" ");
        }
        std::io::Write::flush(&mut std::io::stdout()).ok();
        std::thread::sleep(Duration::from_millis(100));
    }
    println!(" done");

    let graph = build_demo_graph();
    println!("LLM Baseline: ~10 decisions/sec (simulated)\n");

    (graph, 10.0)
}

/// Build demo routing graph using torg_core::Builder
///
/// Implements the "Cognitive Switchboard" routing logic:
/// - Input 0: is_sensitive
/// - Input 1: battery_low
/// - Input 2: complexity_high (unused in simplified logic)
/// - Input 3: peer_available
///
/// Logic:
/// - Node 4: should_delegate = battery_low OR peer_available (simplified from AND)
/// - Node 5: cloud_allowed = NOT is_sensitive
/// - Output: Node 4 (delegate decision)
fn build_demo_graph() -> Graph {
    let mut builder = Builder::new();

    // Declare inputs 0-3
    builder.push(Token::InputDecl).unwrap();
    builder.push(Token::Id(0)).unwrap(); // is_sensitive
    builder.push(Token::InputDecl).unwrap();
    builder.push(Token::Id(1)).unwrap(); // battery_low
    builder.push(Token::InputDecl).unwrap();
    builder.push(Token::Id(2)).unwrap(); // complexity_high
    builder.push(Token::InputDecl).unwrap();
    builder.push(Token::Id(3)).unwrap(); // peer_available

    // Node 4: delegate decision
    // For proper AND: AND(a,b) = NOR(NOT(a), NOT(b))
    // Simplified demo: OR(battery_low, peer_available)
    builder.push(Token::NodeStart).unwrap();
    builder.push(Token::Id(4)).unwrap();
    builder.push(Token::Or).unwrap();
    builder.push(Token::Id(1)).unwrap(); // battery_low
    builder.push(Token::Id(3)).unwrap(); // peer_available
    builder.push(Token::NodeEnd).unwrap();

    // Node 5: cloud_allowed = NOR(is_sensitive, is_sensitive) = NOT is_sensitive
    builder.push(Token::NodeStart).unwrap();
    builder.push(Token::Id(5)).unwrap();
    builder.push(Token::Nor).unwrap();
    builder.push(Token::Id(0)).unwrap();
    builder.push(Token::Id(0)).unwrap();
    builder.push(Token::NodeEnd).unwrap();

    // Output: delegate decision (node 4)
    builder.push(Token::OutputDecl).unwrap();
    builder.push(Token::Id(4)).unwrap();

    builder.finish().unwrap()
}
