//! Co-resident two-model proof: Gemma-4 E2B (primary) and Gemma-4 12B
//! (secondary) loaded into a single `ModelRegistry`, each serving a completion
//! while the other stays resident, then a classify-then-dispatch (router->serve)
//! pass over the same resident models.
//!
//! `model_registry_test.rs` and `concurrent_inference_test.rs` only ever
//! exercise an *empty* registry, so nothing confirms that the two real local
//! models arkavo-router selects (`ModelChoice::LocalGemma4E2B` +
//! `LocalGemma4_12B`) actually coexist in one process and both generate. This
//! test closes that gap.
//!
//! Each model is driven with the *production* sampling config: per-model
//! temperature/top_p/thinking-mode pulled from the canonical
//! `ModelChoice::optimal_sampling()` (mirroring
//! `arkavo_router::provider::sampling_config_for`). Thinking is `Off` for both
//! Gemma-4 models, so the assertions also guard against the Gemma-4 thinking
//! channel leaking control tokens into user-facing content.
//!
//! Run with:
//!   cargo test --features llama-cpp -p arkavo-llm \
//!     --test coresident_e2b_12b_test -- --ignored --nocapture

#![allow(clippy::disallowed_methods)] // tokio::test uses block_on internally

#[cfg(all(test, feature = "llama-cpp"))]
mod tests {
    use arkavo_llm::{LlamaCppProvider, Message, ModelRegistry, Provider, SamplingConfig};
    use arkavo_router::ModelChoice;
    use arkavo_router::classifier::Classification;
    use std::sync::Arc;
    use std::time::Instant;

    /// Resolve a model's cached GGUF from the canonical `ModelChoice` metadata
    /// (`cache_dir_name()` + `gguf_filename()`), scanning the HuggingFace
    /// snapshot folders. Returns None if the model is not in the cache.
    fn resolve(choice: &ModelChoice) -> Option<String> {
        let dir = choice.cache_dir_name()?;
        let file = choice.gguf_filename()?;
        let home = std::env::var("HOME").ok()?;
        let snapshots =
            std::fs::read_dir(format!("{home}/.cache/huggingface/hub/{dir}/snapshots")).ok()?;
        for snapshot in snapshots.flatten() {
            let gguf = snapshot.path().join(file);
            if gguf.exists() {
                return Some(gguf.to_string_lossy().into_owned());
            }
        }
        None
    }

    /// Build the sampling config the production router uses for a model, mirroring
    /// `arkavo_router::provider::sampling_config_for`: per-model temperature,
    /// top_p and thinking-mode from `ModelChoice::optimal_sampling()`. `max_tokens`
    /// is bounded for the test; everything else stays at the production default.
    fn production_config(choice: &ModelChoice) -> SamplingConfig {
        match choice.optimal_sampling() {
            Some((temperature, top_p, thinking)) => SamplingConfig {
                temperature,
                top_p,
                thinking_mode: Some(thinking),
                max_tokens: 64,
                ..Default::default()
            },
            None => SamplingConfig {
                max_tokens: 64,
                ..Default::default()
            },
        }
    }

    /// Generate from a model that is already resident in the registry, using its
    /// production sampling config. Demonstrates serving a chosen model with no
    /// reload or swap.
    async fn serve(registry: &Arc<ModelRegistry>, choice: &ModelChoice, prompt: &str) -> String {
        let provider = LlamaCppProvider::new_with_registry(
            registry.clone(),
            choice.name().to_string(),
            production_config(choice),
        )
        .expect("registry-backed provider");
        provider
            .complete(vec![Message::user(prompt)])
            .await
            .expect("completion failed")
    }

    /// Resident-set size of this process in MiB via `ps` (Unix). Returns None on
    /// platforms where `ps -o rss=` is unavailable so the test stays portable.
    fn rss_mib() -> Option<u64> {
        let pid = std::process::id().to_string();
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &pid])
            .output()
            .ok()?;
        let kb: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
        Some(kb / 1024)
    }

    fn file_size_mib(path: &str) -> u64 {
        std::fs::metadata(path)
            .map(|m| m.len() / (1024 * 1024))
            .unwrap_or(0)
    }

    #[tokio::test]
    #[ignore = "requires local Gemma-4 E2B (~3GB) + 12B (~7GB) co-resident (~10GB RAM)"]
    async fn test_e2b_and_12b_co_resident() {
        let e2b = ModelChoice::LocalGemma4E2B;
        let g12 = ModelChoice::LocalGemma4_12B;

        let (e2b_path, g12_path) = match (resolve(&e2b), resolve(&g12)) {
            (Some(a), Some(b)) => (a, b),
            _ => {
                eprintln!(
                    "Skipping: need both Gemma-4 E2B and Gemma-4 12B GGUFs in the HuggingFace cache"
                );
                return;
            }
        };

        println!("RSS before load: {:?} MiB", rss_mib());

        // Load BOTH models into one registry — the co-residency under test.
        let registry = Arc::new(ModelRegistry::new());

        let t = Instant::now();
        registry.load(e2b.name(), &e2b_path).expect("load E2B");
        let e2b_load_ms = t.elapsed().as_millis();

        let t = Instant::now();
        registry.load(g12.name(), &g12_path).expect("load 12B");
        let g12_load_ms = t.elapsed().as_millis();

        // Co-residency invariant: both models live in the same process at once.
        assert_eq!(registry.len(), 2, "expected exactly two co-resident models");
        assert!(registry.is_loaded(e2b.name()), "E2B is not resident");
        assert!(registry.is_loaded(g12.name()), "12B is not resident");

        println!(
            "Co-resident {:?}: E2B {} MiB ({e2b_load_ms} ms), 12B {} MiB ({g12_load_ms} ms); RSS {:?} MiB",
            registry.model_names(),
            file_size_mib(&e2b_path),
            file_size_mib(&g12_path),
            rss_mib(),
        );

        // Generate from each model while the other stays resident. Decode calls
        // are sequential (one inference at a time on the shared GPU device).
        let g12_resp = serve(
            &registry,
            &g12,
            "What is the capital of France? Answer in one word.",
        )
        .await;
        assert_eq!(
            registry.len(),
            2,
            "a model was evicted while the 12B generated"
        );

        let e2b_resp = serve(&registry, &e2b, "What is 2 + 2? Answer with the number.").await;

        println!("12B: {}", g12_resp.trim());
        println!("E2B: {}", e2b_resp.trim());

        // Thinking is `ThinkingMode::Off` for both Gemma-4 models, so no Gemma-4
        // channel control token may leak into user-facing content.
        assert!(
            !g12_resp.contains("<|channel>"),
            "12B leaked a thinking-channel control token into content: {g12_resp}"
        );
        assert!(
            !e2b_resp.contains("<|channel>"),
            "E2B leaked a thinking-channel control token into content: {e2b_resp}"
        );
        // Both produced correct, clean answers while co-resident.
        assert!(
            g12_resp.to_lowercase().contains("paris"),
            "12B answer unexpected: {g12_resp}"
        );
        assert!(e2b_resp.contains('4'), "E2B answer unexpected: {e2b_resp}");

        // --- router -> serve: classify a prompt, dispatch to the chosen
        // resident model, all without any reload or eviction. Uses the canonical
        // complexity classifier: simple work -> primary (E2B), multi-step work ->
        // secondary (12B), matching arkavo-edge's primary/secondary split.
        let simple = "What is 2 + 2?";
        let complex = "Design a database schema, and then implement the REST API, \
                       after that write integration tests and document the endpoints.";

        for (prompt, expected) in [(simple, &e2b), (complex, &g12)] {
            let (is_complex, score) = Classification::detect_complexity(prompt);
            let chosen = if is_complex { &g12 } else { &e2b };
            println!(
                "route (complex={is_complex}, score={score}) -> {}: {prompt:?}",
                chosen.name()
            );
            assert_eq!(
                chosen.name(),
                expected.name(),
                "router dispatched to the wrong resident model for: {prompt}"
            );

            let resp = serve(&registry, chosen, prompt).await;
            assert!(
                !resp.trim().is_empty(),
                "{} produced no output for the routed prompt",
                chosen.name()
            );
            // Serving the routed model neither reloaded nor evicted anything.
            assert_eq!(registry.len(), 2, "registry changed during routed dispatch");
        }

        assert!(registry.is_loaded(e2b.name()) && registry.is_loaded(g12.name()));
        println!("RSS at end: {:?} MiB", rss_mib());
    }
}
