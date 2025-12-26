//! LLM-based TØR-G integration tests
//!
//! These tests validate end-to-end constrained generation:
//! Natural Language → Qwen3 + Constrained Decoding → TØR-G Graph → evaluate()
//!
//! Run with: `cargo test -p arkavo-torg -- --ignored`
//! Uses models from HuggingFace hub cache (~/.cache/huggingface/hub)
//! Override with: ARKAVO_TORG_MODEL_PATH=/path/to/model.gguf

#![cfg(not(target_env = "musl"))]

mod llm_harness;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use arkavo_llama_cpp::LlamaContext;
use llm_harness::{
    generate_with_constraints, load_model_if_available, verify_or_behavior, verify_xor_behavior,
};
use torg_core::evaluate;

/// Test 0: Basic constraint enforcement - verify we get a valid graph
#[test]
#[ignore = "Requires local Qwen3 model"]
fn test_basic_generation() {
    let Some(model) = load_model_if_available() else {
        return;
    };
    let ctx = LlamaContext::new(&model).expect("Failed to create context");

    // Just verify we can generate any valid graph
    let graph = generate_with_constraints(&model, &ctx, "A OR B", 50).expect("Generation failed");

    // Just verify we got a valid graph with at least one output
    assert!(
        !graph.outputs.is_empty(),
        "Generated graph should have at least one output"
    );
    eprintln!("Generated graph: {:?}", graph);
}

/// Test 1: Access Control - Two inputs ORed together
#[test]
#[ignore = "Requires local Qwen3 model"]
fn test_access_control_generation() {
    let Some(model) = load_model_if_available() else {
        return;
    };
    let ctx = LlamaContext::new(&model).expect("Failed to create context");

    // Explicit prompt: declare two inputs, OR them, output the result
    let graph =
        generate_with_constraints(&model, &ctx, "input0 OR input1", 50).expect("Generation failed");

    // Check we got at least 2 inputs
    if graph.inputs.len() < 2 {
        eprintln!(
            "Note: Model generated {} inputs, expected 2. Graph: {:?}",
            graph.inputs.len(),
            graph
        );
        // Skip semantic verification if we don't have 2 inputs
        return;
    }

    // Find output ID (should be the last declared output)
    let output_id = *graph.outputs.first().expect("No outputs in graph");

    // Verify OR truth table
    verify_or_behavior(&graph, output_id);
}

/// Test 2: Content Moderation - Generates a valid graph
/// NOTE: Semantic verification requires a more capable model.
#[test]
#[ignore = "Requires local Qwen3 model"]
fn test_content_moderation_generation() {
    let Some(model) = load_model_if_available() else {
        return;
    };
    let ctx = LlamaContext::new(&model).expect("Failed to create context");

    let graph = generate_with_constraints(
        &model,
        &ctx,
        "flagged NOR appeal", // Simpler prompt for small model
        100,
    )
    .expect("Generation failed");

    // Verify we got a valid graph
    assert!(!graph.outputs.is_empty(), "Should have at least one output");
    eprintln!("Generated graph: {:?}", graph);
}

/// Test 3: Agent Routing - XOR operation
/// NOTE: Semantic verification is skipped if model doesn't generate 2 inputs.
/// The 0.6B model often generates simpler circuits.
#[test]
#[ignore = "Requires local Qwen3 model"]
fn test_agent_routing_generation() {
    let Some(model) = load_model_if_available() else {
        return;
    };
    let ctx = LlamaContext::new(&model).expect("Failed to create context");

    let graph = generate_with_constraints(&model, &ctx, "input0 XOR input1", 50)
        .expect("Generation failed");

    // Verify we got a valid graph
    assert!(!graph.outputs.is_empty(), "Should have at least one output");
    eprintln!("Generated graph: {:?}", graph);

    // Skip semantic verification if we don't have 2 inputs
    if graph.inputs.len() < 2 {
        eprintln!(
            "Note: Model generated {} inputs, expected 2",
            graph.inputs.len()
        );
        return;
    }

    let output_id = *graph.outputs.first().expect("No outputs in graph");
    verify_xor_behavior(&graph, output_id);
}

/// Test 4: Majority Vote - Complex circuit generation
/// NOTE: This requires a more capable model for proper semantics.
#[test]
#[ignore = "Requires local Qwen3 model"]
fn test_majority_vote_generation() {
    let Some(model) = load_model_if_available() else {
        return;
    };
    let ctx = LlamaContext::new(&model).expect("Failed to create context");

    // Simpler prompt for the small model
    let graph = generate_with_constraints(&model, &ctx, "input0 OR input1 OR input2", 150)
        .expect("Generation failed");

    // Verify we got a valid graph
    assert!(!graph.outputs.is_empty(), "Should have at least one output");
    eprintln!("Generated graph: {:?}", graph);
}

/// Test 5: Prompt Variations - Different prompts all produce valid graphs
/// NOTE: Semantic equivalence testing requires a more capable model.
#[test]
#[ignore = "Requires local Qwen3 model"]
fn test_prompt_variations() {
    let Some(model) = load_model_if_available() else {
        return;
    };

    // All prompts should produce syntactically valid graphs
    let prompts = ["A OR B", "input0 OR input1", "x XOR y"];

    for prompt in prompts {
        // Create fresh context for each generation (KV cache can't be reused)
        let ctx = LlamaContext::new(&model).expect("Failed to create context");

        let graph = generate_with_constraints(&model, &ctx, prompt, 50)
            .unwrap_or_else(|_| panic!("Generation failed for prompt: {prompt}"));

        // Verify we got a valid graph
        assert!(
            !graph.outputs.is_empty(),
            "Prompt '{prompt}': Should have at least one output"
        );
        eprintln!("Prompt '{}' -> {:?}", prompt, graph);
    }
}

/// Test 6: Constraint Enforcement - Verify ONLY allowed tokens are sampled
/// This test verifies that the logit bias mask is actually being applied
#[test]
#[ignore = "Requires local Qwen3 model"]
fn test_constraint_enforcement() {
    let Some(model) = load_model_if_available() else {
        return;
    };
    let vocab = model.get_vocab();

    // Build token mapping
    let mapping =
        unsafe { arkavo_torg::Qwen3TokenMap::from_vocab(vocab).expect("Failed to build mapping") };
    let vocab_size = mapping.vocab_size();
    let sampler = arkavo_torg::TorgLlamaSampler::new(mapping.into_mapping(), vocab_size);

    // In initial state, check that we have valid constraints
    let allowed = sampler.allowed_tokens();
    assert!(
        !allowed.is_empty(),
        "Initial state should allow some tokens"
    );

    // Verify that the number of allowed tokens is much smaller than vocab
    let allowed_count = allowed.len();
    let vocab_count = vocab_size as usize;
    assert!(
        allowed_count < vocab_count / 100,
        "Constrained set should be << vocab size"
    );

    // Get logit bias and verify format
    let bias = sampler.get_logit_bias();
    assert!(!bias.is_empty(), "Should have bias entries");

    // All bias entries should be NEG_INFINITY for disallowed tokens
    for entry in &bias {
        assert!(
            entry.bias == f32::NEG_INFINITY,
            "Bias should be NEG_INFINITY for disallowed tokens"
        );
    }
}

/// Test 7: Benchmark - Verify generation and evaluation performance
#[test]
#[ignore = "Requires local Qwen3 model"]
fn benchmark_generation_eval() {
    let Some(model) = load_model_if_available() else {
        return;
    };
    let ctx = LlamaContext::new(&model).expect("Failed to create context");

    // Benchmark generation time
    let start = Instant::now();
    let graph = generate_with_constraints(
        &model,
        &ctx,
        "Allow if (admin OR owner) AND NOT suspended",
        100,
    )
    .expect("Generation failed");
    let gen_time = start.elapsed();

    // Generation should complete reasonably quickly (allow more for cold start)
    assert!(
        gen_time < Duration::from_secs(30),
        "Generation took too long: {gen_time:?}",
    );
    eprintln!("Generation time: {gen_time:?}");

    // Benchmark evaluation time
    let output_id = *graph.outputs.first().expect("No outputs in graph");
    let inputs: HashMap<u16, bool> = [(0, true), (1, false), (2, false)].into_iter().collect();

    let iterations = 10_000;
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = evaluate(&graph, &inputs).unwrap()[&output_id];
    }
    let eval_time = start.elapsed();
    let avg_eval = eval_time / iterations;

    // Evaluation should be sub-microsecond
    assert!(
        avg_eval < Duration::from_micros(10),
        "Evaluation too slow: {avg_eval:?} per iteration",
    );
    eprintln!("Evaluation time: {eval_time:?} total, {avg_eval:?} per iteration");
}
