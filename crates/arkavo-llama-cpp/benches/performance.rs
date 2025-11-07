#![allow(clippy::disallowed_methods)] // Benchmarks need block_on

use arkavo_llama_cpp::{
    LlamaContext, LlamaModel, apply_chat_template, batch_get_one_with_logits,
    batch_get_one_with_offset, create_sampler_chain, decode_batch, ffi, init_llama_logging,
    token_to_piece, tokenize_with_model,
};
use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use std::ffi::CString;
use std::time::Duration;

/// Test fixture for benchmarking
struct TestFixture {
    model: LlamaModel,
}

impl TestFixture {
    fn new() -> Option<Self> {
        init_llama_logging();

        // Try to find the GGUF model via env var or standard paths
        let model_path = std::env::var("ARKAVO_BENCH_MODEL").ok();

        if let Some(path) = model_path {
            if let Ok(model) = LlamaModel::from_file(&path) {
                eprintln!("✓ Loaded model from ARKAVO_BENCH_MODEL: {}", path);
                return Some(Self { model });
            }
        }

        // Try common HuggingFace cache locations
        let home = std::env::var("HOME").ok()?;
        let hf_patterns = [
            format!(
                "{}/.cache/huggingface/hub/models--*gemma*270m*GGUF*/snapshots/*/*.gguf",
                home
            ),
            format!(
                "{}/.cache/huggingface/hub/models--*gemma*GGUF*/snapshots/*/*.gguf",
                home
            ),
        ];

        for pattern in &hf_patterns {
            if let Ok(entries) = glob::glob(pattern) {
                for entry in entries.flatten() {
                    if let Ok(model) = LlamaModel::from_file(entry.to_str().unwrap()) {
                        eprintln!("✓ Loaded model: {}", entry.display());
                        return Some(Self { model });
                    }
                }
            }
        }

        eprintln!("Warning: No model found for benchmarking.");
        eprintln!("Set ARKAVO_BENCH_MODEL=/path/to/model.gguf to run benchmarks.");
        None
    }
}

/// Benchmark context creation
fn bench_context_creation(c: &mut Criterion) {
    let fixture = match TestFixture::new() {
        Some(f) => f,
        None => return,
    };

    c.bench_function("context_creation", |b| {
        b.iter(|| {
            let ctx = LlamaContext::new(black_box(&fixture.model));
            assert!(ctx.is_ok());
            drop(ctx);
        });
    });
}

/// Benchmark tokenization speed
fn bench_tokenization(c: &mut Criterion) {
    let fixture = match TestFixture::new() {
        Some(f) => f,
        None => return,
    };

    let vocab = fixture.model.get_vocab();

    let prompts = vec![
        ("short", "Hello, world!"),
        (
            "medium",
            "Explain quantum computing in simple terms. What are the key principles?",
        ),
        (
            "long",
            "Write a detailed essay about the history of artificial intelligence, covering major milestones from the Turing test to modern large language models. Discuss key breakthroughs, challenges, and future directions.",
        ),
    ];

    let mut group = c.benchmark_group("tokenization");
    for (name, prompt) in prompts {
        group.bench_with_input(BenchmarkId::from_parameter(name), &prompt, |b, prompt| {
            b.iter(|| {
                let tokens = tokenize_with_model(vocab, black_box(prompt.as_bytes()));
                assert!(tokens.is_ok());
            });
        });
    }
    group.finish();
}

/// Benchmark chat template application
fn bench_chat_template(c: &mut Criterion) {
    let messages = vec![
        ffi::llama_chat_message {
            role: CString::new("user").unwrap().as_ptr(),
            content: CString::new("Hello, how are you?").unwrap().as_ptr(),
        },
        ffi::llama_chat_message {
            role: CString::new("assistant").unwrap().as_ptr(),
            content: CString::new("I'm doing well, thank you!").unwrap().as_ptr(),
        },
        ffi::llama_chat_message {
            role: CString::new("user").unwrap().as_ptr(),
            content: CString::new("Can you explain machine learning?")
                .unwrap()
                .as_ptr(),
        },
    ];

    c.bench_function("chat_template_application", |b| {
        b.iter(|| {
            let result = apply_chat_template(black_box(&messages), true);
            assert!(result.is_ok());
        });
    });
}

/// Benchmark batch processing with different chunk sizes
fn bench_batch_processing(c: &mut Criterion) {
    let fixture = match TestFixture::new() {
        Some(f) => f,
        None => return,
    };

    let ctx = match LlamaContext::new(&fixture.model) {
        Ok(c) => c,
        Err(_) => return,
    };

    let vocab = fixture.model.get_vocab();
    let prompt = "Explain quantum mechanics in detail";
    let tokens = match tokenize_with_model(vocab, prompt.as_bytes()) {
        Ok(t) => t,
        Err(_) => return,
    };

    let chunk_sizes = vec![16, 32, 64, 128];
    let mut group = c.benchmark_group("batch_processing");

    for chunk_size in chunk_sizes {
        group.bench_with_input(
            BenchmarkId::from_parameter(chunk_size),
            &chunk_size,
            |b, &chunk_size| {
                b.iter(|| {
                    let mut pos_offset = 0i32;
                    for chunk in tokens.chunks(chunk_size) {
                        let batch = batch_get_one_with_offset(chunk, pos_offset, false);
                        let _ = decode_batch(&ctx, batch);
                        pos_offset += i32::try_from(chunk.len()).unwrap_or(i32::MAX);
                    }
                });
            },
        );
    }
    group.finish();
}

/// Benchmark single token generation (measures sampling overhead)
fn bench_single_token_generation(c: &mut Criterion) {
    let fixture = match TestFixture::new() {
        Some(f) => f,
        None => return,
    };

    let ctx = match LlamaContext::new(&fixture.model) {
        Ok(c) => c,
        Err(_) => return,
    };

    let vocab = fixture.model.get_vocab();
    let prompt = "Hello";
    let tokens = match tokenize_with_model(vocab, prompt.as_bytes()) {
        Ok(t) => t,
        Err(_) => return,
    };

    // Process prompt
    let batch = batch_get_one_with_logits(&tokens, true);
    let _ = decode_batch(&ctx, batch);

    let sampler = match create_sampler_chain(0.7, 0.9, 40, 42) {
        Ok(s) => s,
        Err(_) => return,
    };

    c.bench_function("single_token_generation", |b| {
        b.iter(|| {
            let token = sampler.sample(black_box(&ctx), -1);
            let _ = token_to_piece(vocab, token, false);
        });
    });
}

/// Benchmark time to first token (TTFT)
fn bench_time_to_first_token(c: &mut Criterion) {
    let fixture = match TestFixture::new() {
        Some(f) => f,
        None => return,
    };

    let prompts = vec![
        ("short", "Hi"),
        ("medium", "Explain machine learning"),
        (
            "long",
            "Write a comprehensive guide to Rust programming covering ownership, borrowing, and lifetimes",
        ),
    ];

    let mut group = c.benchmark_group("time_to_first_token");
    for (name, prompt) in prompts {
        group.bench_with_input(BenchmarkId::from_parameter(name), &prompt, |b, prompt| {
            b.iter(|| {
                // Create fresh context for each iteration
                let ctx = LlamaContext::new(black_box(&fixture.model)).unwrap();
                let vocab = fixture.model.get_vocab();

                // Tokenize
                let tokens = tokenize_with_model(vocab, prompt.as_bytes()).unwrap();

                // Process all tokens to get first output
                let batch = batch_get_one_with_logits(&tokens, true);
                let _ = decode_batch(&ctx, batch);

                // Sample first token
                let sampler = create_sampler_chain(0.7, 0.9, 40, 42).unwrap();
                let _ = sampler.sample(&ctx, -1);
            });
        });
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(20))
        .sample_size(50)
        .warm_up_time(Duration::from_secs(3));
    targets = bench_context_creation,
              bench_tokenization,
              bench_chat_template,
              bench_batch_processing,
              bench_single_token_generation,
              bench_time_to_first_token
}
criterion_main!(benches);
