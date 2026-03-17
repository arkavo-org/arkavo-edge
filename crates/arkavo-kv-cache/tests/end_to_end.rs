//! End-to-end integration test for composable KV cache context slots.
//!
//! Requires a real GGUF model file. Run with:
//!
//! ```sh
//! LLAMA_MODEL=~/models/Ministral-3B-Instruct-2501-Q6_K.gguf \
//!   cargo test -p arkavo-kv-cache --features llama-cpp --test end_to_end \
//!   -- --ignored --nocapture
//! ```
//!
//! Validates: .bin file creation, multi-slot loading with position shifting,
//! seq_cp + offset decoding, model attention to composed context, and
//! unload/compaction correctness.

#![cfg(all(feature = "llama-cpp", not(target_env = "musl")))]

use arkavo_kv_cache::ContextManager;
use arkavo_llama_cpp::{
    LlamaContext, LlamaModel, batch_free, batch_init_with_tokens_seq, create_sampler_chain,
    decode_batch, init_llama_logging, set_debug_logging, token_to_bytes, tokenize_with_model,
};

const LEARNING_SEQ: i32 = 0;
const CONVERSATION_SEQ: i32 = 1;

/// Greedy sampling loop: decode one token at a time on `seq_id`, accumulate text.
fn generate(
    ctx: &LlamaContext,
    model: &LlamaModel,
    seq_id: i32,
    start_pos: i32,
    max_tokens: usize,
) -> String {
    let vocab = model.get_vocab();
    let eos = model.get_eos_token();
    // temp=0 → greedy for deterministic output
    let sampler = create_sampler_chain(0.0, 1.0, 40, 42).expect("sampler creation failed");

    let mut output = String::new();
    let mut pos = start_pos;

    for _ in 0..max_tokens {
        let token = sampler.sample(ctx, -1);
        if token == eos {
            break;
        }
        sampler.accept(token);

        if let Ok(bytes) = token_to_bytes(vocab, token, false) {
            output.push_str(&String::from_utf8_lossy(&bytes));
        }

        let mut batch = batch_init_with_tokens_seq(&[token], pos, seq_id, true);
        decode_batch(ctx, batch).expect("decode failed during generation");
        batch_free(&mut batch);

        pos += 1;
    }

    output
}

#[test]
#[ignore] // requires model file
fn end_to_end_context_slots() {
    let model_path = std::env::var("LLAMA_MODEL")
        .unwrap_or_else(|_| panic!("Set LLAMA_MODEL=/path/to/model.gguf"));

    init_llama_logging();
    set_debug_logging(std::env::var("ARKAVO_DEBUG").is_ok());

    let model = LlamaModel::from_file(&model_path).expect("Failed to load model");
    let ctx = LlamaContext::new_with_sequences(&model, 2, true).expect("Failed to create context");
    let vocab = model.get_vocab();
    let mem = ctx.get_memory();
    let dir = tempfile::tempdir().unwrap();

    // --- Stage 1: Build .bin files from text ---

    let text_a = "The user's name is Alice. Always address her by name.";
    let tokens_a = tokenize_with_model(vocab, text_a.as_bytes()).expect("tokenize A failed");
    let mut batch_a = batch_init_with_tokens_seq(&tokens_a, 0, LEARNING_SEQ, true);
    decode_batch(&ctx, batch_a).expect("decode A failed");
    batch_free(&mut batch_a);

    let path_a = dir.path().join("name.bin");
    ctx.state_seq_save_file(path_a.to_str().unwrap(), LEARNING_SEQ, &tokens_a)
        .expect("save slot A failed");

    // Clear and encode second fact
    mem.seq_rm(LEARNING_SEQ, -1, -1);

    let text_b = "Alice is a compiler engineer who works on LLVM.";
    let tokens_b = tokenize_with_model(vocab, text_b.as_bytes()).expect("tokenize B failed");
    let mut batch_b = batch_init_with_tokens_seq(&tokens_b, 0, LEARNING_SEQ, true);
    decode_batch(&ctx, batch_b).expect("decode B failed");
    batch_free(&mut batch_b);

    let path_b = dir.path().join("occupation.bin");
    ctx.state_seq_save_file(path_b.to_str().unwrap(), LEARNING_SEQ, &tokens_b)
        .expect("save slot B failed");

    // Clear everything — fresh start for composition
    mem.seq_rm(LEARNING_SEQ, -1, -1);
    mem.seq_rm(CONVERSATION_SEQ, -1, -1);

    eprintln!("--- Stage 1 complete: .bin files created ---");
    eprintln!("  name.bin: {} tokens", tokens_a.len());
    eprintln!("  occupation.bin: {} tokens", tokens_b.len());

    // --- Stage 2: Load and compose via ContextManager ---

    let mut cm = ContextManager::new(LEARNING_SEQ, CONVERSATION_SEQ);
    cm.load(
        &ctx,
        "name",
        path_a.to_str().unwrap(),
        arkavo_kv_cache::TrustTier::Local,
    )
    .expect("load slot A failed");
    cm.load(
        &ctx,
        "occupation",
        path_b.to_str().unwrap(),
        arkavo_kv_cache::TrustTier::Local,
    )
    .expect("load slot B failed");

    // Verify slots are contiguous, not overlapping
    assert_eq!(cm.slots().len(), 2);
    assert_eq!(cm.slots()[0].pos_start, 0);
    assert_eq!(
        cm.slots()[1].pos_start,
        cm.slots()[0].pos_end,
        "Slots must be contiguous: slot B start ({}) should equal slot A end ({})",
        cm.slots()[1].pos_start,
        cm.slots()[0].pos_end
    );

    eprintln!("--- Stage 2 complete: slots loaded ---");
    eprintln!(
        "  slot 'name': [{}, {})",
        cm.slots()[0].pos_start,
        cm.slots()[0].pos_end
    );
    eprintln!(
        "  slot 'occupation': [{}, {})",
        cm.slots()[1].pos_start,
        cm.slots()[1].pos_end
    );

    let offset = cm
        .begin_conversation(&ctx)
        .expect("begin_conversation failed");
    assert_eq!(offset, cm.conversation_offset());

    // --- Stage 3: Generate ---

    let prompt = "What is my name and what do I work on?";
    let prompt_tokens = tokenize_with_model(vocab, prompt.as_bytes()).expect("tokenize Q failed");
    let mut batch_q = batch_init_with_tokens_seq(&prompt_tokens, offset, CONVERSATION_SEQ, true);
    decode_batch(&ctx, batch_q).expect("decode prompt failed");
    batch_free(&mut batch_q);

    let gen_start = offset + prompt_tokens.len() as i32;
    let output = generate(&ctx, &model, CONVERSATION_SEQ, gen_start, 64);

    eprintln!("--- Stage 3 complete: generation ---");
    eprintln!("  Prompt: {prompt}");
    eprintln!("  Output: {output}");

    // --- Stage 4: Verify model attends to composed context ---

    let response = output.to_lowercase();
    assert!(
        response.contains("alice"),
        "Model should reference 'Alice' from slot 'name'. Got: {output}"
    );
    assert!(
        response.contains("compiler") || response.contains("llvm"),
        "Model should reference occupation from slot 'occupation'. Got: {output}"
    );

    eprintln!("--- Stage 4 complete: content verified ---");

    // --- Stage 5: Verify slot independence after unload ---

    cm.end_conversation(&ctx).expect("end_conversation failed");
    cm.unload(&ctx, "occupation")
        .expect("unload occupation failed");

    let offset2 = cm
        .begin_conversation(&ctx)
        .expect("begin_conversation 2 failed");
    assert!(
        offset2 < offset,
        "Offset should shrink after unload: {} should be < {}",
        offset2,
        offset
    );

    let mut batch_q2 = batch_init_with_tokens_seq(&prompt_tokens, offset2, CONVERSATION_SEQ, true);
    decode_batch(&ctx, batch_q2).expect("decode prompt 2 failed");
    batch_free(&mut batch_q2);

    let gen_start2 = offset2 + prompt_tokens.len() as i32;
    let output2 = generate(&ctx, &model, CONVERSATION_SEQ, gen_start2, 64);

    eprintln!("--- Stage 5 complete: after unload ---");
    eprintln!("  Output: {output2}");

    let response2 = output2.to_lowercase();
    assert!(
        response2.contains("alice"),
        "Should still know name after unloading occupation. Got: {output2}"
    );
    // Occupation assertion intentionally omitted — model may or may not mention it

    cm.end_conversation(&ctx)
        .expect("end_conversation final failed");

    eprintln!("--- All stages passed ---");
}
