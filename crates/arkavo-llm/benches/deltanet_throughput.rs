//! DeltaNet Throughput Benchmark (Phase 0 Profiling)
//!
//! Measures Qwen3.5 autoregressive generation throughput to establish baseline
//! for the fused DeltaNet Metal kernel optimization.
//!
//! Requires the `llama-cpp` feature and a Qwen3.5 GGUF model file.
//! Set ARKAVO_BENCH_MODEL to the model path, or it will skip.
//!
//! Usage:
//!   ARKAVO_BENCH_MODEL=/path/to/qwen3.5.gguf cargo bench --bench deltanet_throughput --features llama-cpp
//!
//! For GPU trace analysis:
//!   GGML_METAL_CAPTURE_ENABLE=1 ARKAVO_BENCH_MODEL=/path/to/qwen3.5.gguf cargo bench --bench deltanet_throughput --features llama-cpp

use criterion::{Criterion, criterion_group, criterion_main};
use std::time::Duration;

#[cfg(feature = "llama-cpp")]
mod bench {
    use super::*;
    use arkavo_llama_cpp::{
        LlamaContext, LlamaModel, batch_get_one, create_sampler_chain, decode_batch, perf_context,
        perf_context_print, perf_context_reset, tokenize_with_model,
    };

    fn get_model_path() -> Option<String> {
        std::env::var("ARKAVO_BENCH_MODEL").ok()
    }

    pub(crate) fn deltanet_throughput_bench(c: &mut Criterion) {
        let model_path = match get_model_path() {
            Some(p) => p,
            None => {
                eprintln!("ARKAVO_BENCH_MODEL not set, skipping DeltaNet throughput benchmark");
                return;
            }
        };

        let model = match LlamaModel::from_file(&model_path) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Failed to load model: {}", e);
                return;
            }
        };

        let mut group = c.benchmark_group("deltanet_throughput");
        group.sample_size(10);
        group.measurement_time(Duration::from_secs(30));

        group.bench_function("generate_100_tokens", |b| {
            b.iter(|| {
                let mut ctx = LlamaContext::new(&model).expect("context creation");
                perf_context_reset(&mut ctx);

                let prompt = b"Explain the concept of delta rules in neural networks";
                let tokens = tokenize_with_model(model.get_vocab(), prompt)
                    .expect("tokenization");

                let batch = batch_get_one(&tokens);
                decode_batch(&ctx, batch).expect("prompt eval");

                let sampler = create_sampler_chain(0.7, 0.9, 40, 42)
                    .expect("sampler creation");

                let eos = model.get_eos_token();
                for i in 0..100 {
                    let token = sampler.sample(&ctx, -1);
                    if token == eos {
                        break;
                    }
                    let next_batch =
                        batch_get_one(&[token]);
                    decode_batch(&ctx, next_batch).expect("token eval");
                    let _ = i;
                }

                perf_context_print(&ctx);
                let metrics = perf_context(&ctx);
                eprintln!(
                    "[rust] tok/s: {:.1}, TTFT: {:.1}ms, n_eval: {}, n_p_eval: {}, t_eval_ms: {:.1}, t_p_eval_ms: {:.1}",
                    metrics.tok_per_sec(),
                    metrics.ttft_ms(),
                    metrics.n_eval,
                    metrics.n_p_eval,
                    metrics.t_eval_ms,
                    metrics.t_p_eval_ms,
                );
            });
        });

        group.finish();
    }
}

#[cfg(feature = "llama-cpp")]
criterion_group!(benches, bench::deltanet_throughput_bench);

#[cfg(not(feature = "llama-cpp"))]
fn dummy_bench(_c: &mut Criterion) {
    eprintln!("llama-cpp feature not enabled, skipping DeltaNet benchmark");
}

#[cfg(not(feature = "llama-cpp"))]
criterion_group!(benches, dummy_bench);

criterion_main!(benches);
