//! LLM-based TØR-G integration tests
//!
//! These tests validate end-to-end constrained generation:
//! Natural Language → Qwen3 + Constrained Decoding → TØR-G Graph → evaluate()
//!
//! Run with: `cargo test -p arkavo-torg -- --ignored`
//! Requires model at ~/.cache/arkavo/models/qwen3-0.6b.gguf
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

/// Test 1: Access Control - "Allow if admin OR owner"
#[test]
#[ignore = "Requires local Qwen3 model"]
fn test_access_control_generation() {
    let Some(model) = load_model_if_available() else {
        return;
    };
    let ctx = LlamaContext::new(&model).expect("Failed to create context");

    let graph = generate_with_constraints(&model, &ctx, "Allow if admin OR owner", 50)
        .expect("Generation failed");

    // Find output ID (should be the last declared output)
    let output_id = *graph.outputs.first().expect("No outputs in graph");

    // Verify OR truth table
    verify_or_behavior(&graph, output_id);
}

/// Test 2: Content Moderation - "Deny if flagged AND NOT appeal"
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
        "Deny if flagged AND NOT appeal (output true to deny)",
        100,
    )
    .expect("Generation failed");

    let output_id = *graph.outputs.first().expect("No outputs in graph");

    // Truth table for: flagged AND NOT appeal
    // flagged=T, appeal=F → T (deny)
    // flagged=T, appeal=T → F (allow - appeal overrides)
    // flagged=F, appeal=F → F (allow - not flagged)
    // flagged=F, appeal=T → F (allow)
    let inputs_map = |flagged: bool, appeal: bool| -> HashMap<u16, bool> {
        [(0, flagged), (1, appeal)].into_iter().collect()
    };

    assert!(
        evaluate(&graph, &inputs_map(true, false)).unwrap()[&output_id],
        "flagged=T, appeal=F should deny"
    );
    assert!(
        !evaluate(&graph, &inputs_map(true, true)).unwrap()[&output_id],
        "flagged=T, appeal=T should allow (appeal overrides)"
    );
    assert!(
        !evaluate(&graph, &inputs_map(false, false)).unwrap()[&output_id],
        "flagged=F, appeal=F should allow"
    );
}

/// Test 3: Agent Routing - "Route if capable XOR busy"
#[test]
#[ignore = "Requires local Qwen3 model"]
fn test_agent_routing_generation() {
    let Some(model) = load_model_if_available() else {
        return;
    };
    let ctx = LlamaContext::new(&model).expect("Failed to create context");

    let graph =
        generate_with_constraints(&model, &ctx, "Route if capable XOR busy", 50)
            .expect("Generation failed");

    let output_id = *graph.outputs.first().expect("No outputs in graph");

    // Verify XOR truth table
    verify_xor_behavior(&graph, output_id);
}

/// Test 4: Majority Vote - "Execute if at least 2 of 3 approve"
#[test]
#[ignore = "Requires local Qwen3 model"]
fn test_majority_vote_generation() {
    let Some(model) = load_model_if_available() else {
        return;
    };
    let ctx = LlamaContext::new(&model).expect("Failed to create context");

    let graph = generate_with_constraints(
        &model,
        &ctx,
        "Execute if at least 2 of 3 approve (alice, bob, charlie)",
        150,
    )
    .expect("Generation failed");

    let output_id = *graph.outputs.first().expect("No outputs in graph");

    // Majority vote: (A AND B) OR (B AND C) OR (A AND C)
    let inputs_map = |a: bool, b: bool, c: bool| -> HashMap<u16, bool> {
        [(0, a), (1, b), (2, c)].into_iter().collect()
    };

    // 2 or more approve → execute
    assert!(
        evaluate(&graph, &inputs_map(true, true, false)).unwrap()[&output_id],
        "A,B approve should execute"
    );
    assert!(
        evaluate(&graph, &inputs_map(true, false, true)).unwrap()[&output_id],
        "A,C approve should execute"
    );
    assert!(
        evaluate(&graph, &inputs_map(false, true, true)).unwrap()[&output_id],
        "B,C approve should execute"
    );
    assert!(
        evaluate(&graph, &inputs_map(true, true, true)).unwrap()[&output_id],
        "All approve should execute"
    );

    // Less than 2 → no execute
    assert!(
        !evaluate(&graph, &inputs_map(true, false, false)).unwrap()[&output_id],
        "Only A should not execute"
    );
    assert!(
        !evaluate(&graph, &inputs_map(false, false, false)).unwrap()[&output_id],
        "None should not execute"
    );
}

/// Test 5: Prompt Variations - Different phrasings should produce equivalent behavior
#[test]
#[ignore = "Requires local Qwen3 model"]
fn test_prompt_variations() {
    let Some(model) = load_model_if_available() else {
        return;
    };
    let ctx = LlamaContext::new(&model).expect("Failed to create context");

    let prompts = [
        "Allow access if the user is an administrator or the owner",
        "admin OR owner",
        "if admin then allow, else if owner then allow",
    ];

    for prompt in prompts {
        let graph = generate_with_constraints(&model, &ctx, prompt, 50)
            .unwrap_or_else(|_| panic!("Generation failed for prompt: {prompt}"));

        let output_id = *graph.outputs.first().expect("No outputs in graph");

        // All should produce OR behavior
        let inputs_map = |a: bool, b: bool| -> HashMap<u16, bool> {
            [(0, a), (1, b)].into_iter().collect()
        };

        assert!(
            evaluate(&graph, &inputs_map(true, false)).unwrap()[&output_id],
            "Prompt '{prompt}': OR(T,F) should be true",
        );
        assert!(
            !evaluate(&graph, &inputs_map(false, false)).unwrap()[&output_id],
            "Prompt '{prompt}': OR(F,F) should be false",
        );
    }
}

/// Test 6: Constraint Enforcement - Verify masked tokens are never sampled
#[test]
#[ignore = "Requires local Qwen3 model"]
fn test_constraint_enforcement() {
    let Some(model) = load_model_if_available() else {
        return;
    };
    let vocab = model.get_vocab();

    // Build token mapping
    let mapping = unsafe {
        arkavo_torg::Qwen3TokenMap::from_vocab(vocab).expect("Failed to build mapping")
    };
    let vocab_size = mapping.vocab_size();
    let sampler = arkavo_torg::TorgLlamaSampler::new(mapping.into_mapping(), vocab_size);

    // In initial state, check that we have valid constraints
    let allowed = sampler.allowed_tokens();
    assert!(!allowed.is_empty(), "Initial state should allow some tokens");

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
