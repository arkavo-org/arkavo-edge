use crate::{Error, Message, Provider, Result, Role, StreamResponse};
use arkavo_llama_cpp::{
    LlamaContext, LlamaModel, apply_chat_template, batch_free, batch_get_one_with_logits,
    batch_get_one_with_offset, batch_init_with_tokens, create_sampler_chain, decode_batch, ffi,
    init_llama_logging, test_minimal_init, token_to_piece, tokenize_with_model,
};
use async_trait::async_trait;
use std::ffi::CString;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tokio_stream::{Stream, wrappers::UnboundedReceiverStream};

// Debug flag controlled by ARKAVO_DEBUG_CHAT environment variable
static DEBUG_LLAMACPP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Debug, Clone)]
pub struct SamplingConfig {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub max_tokens: u32,
    pub seed: u32,
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            top_p: 0.9,
            top_k: 40,
            max_tokens: 4096, // Default to 4K tokens
            seed: 42,
        }
    }
}

pub struct LlamaCppProvider {
    model: Arc<LlamaModel>,
    context: Arc<Mutex<LlamaContext>>,
    name: String,
    config: SamplingConfig,
}

impl LlamaCppProvider {
    pub fn new(model_name: String, model_path: String) -> Result<Self> {
        Self::new_with_config(model_name, model_path, SamplingConfig::default())
    }

    pub fn new_with_config(
        model_name: String,
        model_path: String,
        config: SamplingConfig,
    ) -> Result<Self> {
        // Initialize debug flag from environment variable once
        if std::env::var("ARKAVO_DEBUG_CHAT").unwrap_or_default() == "1" {
            DEBUG_LLAMACPP.store(true, std::sync::atomic::Ordering::Relaxed);
        }

        // Initialize llama.cpp logging (will check ARKAVO_DEBUG_CHAT internally)
        init_llama_logging();

        // Run minimal FFI test first to catch early crashes
        test_minimal_init()
            .map_err(|e| Error::Config(format!("FFI initialization test failed: {}", e)))?;

        let model = LlamaModel::from_file(&model_path)
            .map_err(|e| Error::Config(format!("Failed to load model: {}", e)))?;

        let context = LlamaContext::new(&model)
            .map_err(|e| Error::Config(format!("Failed to create context: {}", e)))?;

        Ok(Self {
            model: Arc::new(model),
            context: Arc::new(Mutex::new(context)),
            name: model_name,
            config,
        })
    }

    async fn generate_streaming(
        &self,
        messages: Vec<Message>,
    ) -> Result<UnboundedReceiverStream<Result<StreamResponse>>> {
        // Prepare data outside of spawn to avoid Send issues with raw pointers
        let (llama_messages, _cstrings) = Self::messages_to_llama_chat_static(&messages)?;

        // Apply chat template
        let prompt_bytes = apply_chat_template(&llama_messages, true)
            .map_err(|e| Error::Config(format!("Failed to apply chat template: {}", e)))?;

        // Debug: show what the template produced
        if DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed) {
            if let Ok(prompt_str) = std::str::from_utf8(&prompt_bytes) {
                eprintln!("Chat template output:\n{}", prompt_str);
                // Check if it's using the right format
                if prompt_str.contains("<|im_start|>") {
                    eprintln!("WARNING: Template is using Llama-3 format, not Gemma-3!");
                } else if prompt_str.contains("<start_of_turn>") {
                    eprintln!("✓ Template is using correct Gemma-3 format");
                }
            }
        }

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let model = self.model.clone();
        let context = self.context.clone();
        let config = self.config.clone();

        tokio::spawn(async move {
            let start_time = Instant::now();
            let mut first_token_time: Option<Instant> = None;
            let mut tokens_generated = 0u32;

            let result = async {
                let ctx = context.lock().await;

                // Clear KV cache for fresh conversation - prevents position mismatch errors
                ctx.clear_kv_cache();

                // Get vocab and tokenize inside the lock to avoid Send issues
                let vocab = model.get_vocab();
                let input_tokens = tokenize_with_model(vocab, &prompt_bytes)
                    .map_err(|e| Error::Config(format!("Failed to tokenize: {}", e)))?;

                tracing::info!("Input tokenized to {} tokens", input_tokens.len());
                if DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed) {
                    eprintln!(
                        "🔍 First 10 tokens: {:?}",
                        input_tokens.iter().take(10).collect::<Vec<_>>()
                    );
                }

                // Create sampler
                let sampler = create_sampler_chain(
                    config.temperature,
                    config.top_p,
                    config.top_k,
                    config.seed,
                )
                .map_err(|e| Error::Config(format!("Failed to create sampler: {}", e)))?;

                // Get EOS token from the model's vocabulary
                let eos_token = model.get_eos_token();
                if DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed) {
                    eprintln!("EOS token from model: {}", eos_token);
                }

                // Process input tokens - CRUCIAL: request logits on final token for sampling
                if DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed) {
                    eprintln!(
                        "🔥 About to decode input batch with {} tokens",
                        input_tokens.len()
                    );
                }

                // Try smaller batch processing to avoid hanging issues
                if input_tokens.len() > 32 {
                    // Process tokens in smaller chunks with proper position offsets
                    let chunk_size = 16;
                    let mut pos_offset = 0i32;
                    for (i, chunk) in input_tokens.chunks(chunk_size).enumerate() {
                        if DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed) {
                            eprintln!(
                                "  📦 Processing chunk {} with {} tokens (pos_offset: {})",
                                i + 1,
                                chunk.len(),
                                pos_offset
                            );
                        }
                        let is_last_chunk = (i + 1) * chunk_size >= input_tokens.len();
                        let batch = batch_get_one_with_offset(chunk, pos_offset, is_last_chunk);
                        decode_batch(&ctx, batch).map_err(|e| {
                            Error::Config(format!("Failed to decode chunk {}: {}", i + 1, e))
                        })?;
                        pos_offset += chunk.len() as i32;
                    }
                } else {
                    let batch = batch_get_one_with_logits(&input_tokens, true);
                    decode_batch(&ctx, batch)
                        .map_err(|e| Error::Config(format!("Failed to decode input: {}", e)))?;
                }
                if DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed) {
                    eprintln!("✅ Input batch decoded successfully");
                }

                // Generation loop - limit to reasonable context window
                let max_generation = std::cmp::min(config.max_tokens, 30000); // Max generation within 32K context
                let mut pos = input_tokens.len() as i32; // Track position for new tokens

                if DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed) {
                    eprintln!(
                        "🎯 Max generation tokens: {} (config.max_tokens: {})",
                        max_generation, config.max_tokens
                    );
                }

                for i in 0..max_generation {
                    // Guardrail: assert logits exist before sampling
                    let logits_ptr = ctx.get_logits_ith(-1);
                    if logits_ptr.is_null() {
                        return Err(Error::Config(
                            "No logits available for sampling - decode step missing logits=1"
                                .to_string(),
                        ));
                    }
                    if DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed) {
                        eprintln!(
                            "✓ Logits available for sampling token #{} at pos {}",
                            i + 1,
                            pos
                        );
                    }

                    // Sample next token (using -1 for last logits)
                    let token = sampler.sample(&ctx, -1);
                    if DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed) {
                        eprintln!("Sampled token: {} at pos {}", token, pos);
                    }

                    // Check for EOS
                    if token == eos_token {
                        tracing::info!("Generation stopped at EOS token");
                        break;
                    }

                    // Convert token to text
                    let piece = token_to_piece(vocab, token, false)
                        .map_err(|e| Error::Config(format!("Failed to decode token: {}", e)))?;

                    // Record first token timing
                    if first_token_time.is_none() {
                        first_token_time = Some(Instant::now());
                    }

                    tokens_generated += 1;

                    // Send the token
                    let response = StreamResponse {
                        content: piece,
                        done: false, // Will be set to true when we break
                    };

                    if tx.send(Ok(response)).is_err() {
                        break; // Receiver dropped
                    }

                    // Accept the token for the sampler (for repetition penalty etc)
                    sampler.accept(token);

                    // CRITICAL: Feed the token back to the model and decode to get new logits
                    // Use batch_init_with_tokens for proper position handling
                    let mut batch = batch_init_with_tokens(&[token], pos, true);
                    decode_batch(&ctx, batch).map_err(|e| {
                        Error::Config(format!("Failed to decode token at pos {}: {}", pos, e))
                    })?;
                    batch_free(&mut batch); // Clean up the batch

                    // Increment position for next token
                    pos += 1;

                    // Check if we're approaching context limit
                    let total_tokens = input_tokens.len() + tokens_generated as usize;
                    if total_tokens >= 32000 {
                        // Leave some room before hitting the 32768 limit
                        tracing::info!(
                            "Generation stopped at context limit: {} tokens",
                            total_tokens
                        );
                        let _ = tx.send(Ok(StreamResponse {
                            content: String::new(),
                            done: true,
                        }));
                        break;
                    }
                }

                Ok::<(), Error>(())
            }
            .await;

            // Send final metrics or error
            match result {
                Ok(()) => {
                    let total_time = start_time.elapsed();
                    let ttft = first_token_time.map(|t| t.duration_since(start_time));
                    let tokens_per_sec = if total_time.as_secs_f64() > 0.0 {
                        tokens_generated as f64 / total_time.as_secs_f64()
                    } else {
                        0.0
                    };

                    let metrics_msg = if let Some(ttft) = ttft {
                        format!(
                            "\nGenerated {} tokens in {:.2}s ({:.1} tok/s, TTFT: {}ms)",
                            tokens_generated,
                            total_time.as_secs_f64(),
                            tokens_per_sec,
                            ttft.as_millis()
                        )
                    } else {
                        format!(
                            "\nGenerated {} tokens in {:.2}s ({:.1} tok/s)",
                            tokens_generated,
                            total_time.as_secs_f64(),
                            tokens_per_sec
                        )
                    };

                    tracing::info!("{}", metrics_msg);
                    eprintln!("{}", metrics_msg);

                    // Send final done response
                    let _ = tx.send(Ok(StreamResponse {
                        content: String::new(),
                        done: true,
                    }));
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
        });

        Ok(UnboundedReceiverStream::new(rx))
    }

    fn messages_to_llama_chat_static(
        messages: &[Message],
    ) -> Result<(Vec<ffi::llama_chat_message>, Vec<CString>)> {
        let mut llama_messages = Vec::new();
        let mut role_strings = Vec::new();
        let mut content_strings = Vec::new();

        for msg in messages {
            let role_str = match msg.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
            };

            let role_cstring = CString::new(role_str)
                .map_err(|e| Error::Config(format!("Invalid role string: {}", e)))?;
            let content_cstring = CString::new(msg.content.clone())
                .map_err(|e| Error::Config(format!("Invalid content string: {}", e)))?;

            llama_messages.push(ffi::llama_chat_message {
                role: role_cstring.as_ptr(),
                content: content_cstring.as_ptr(),
            });

            role_strings.push(role_cstring);
            content_strings.push(content_cstring);
        }

        // Store all CStrings together to keep them alive
        let mut all_cstrings = role_strings;
        all_cstrings.extend(content_strings);

        Ok((llama_messages, all_cstrings))
    }
}

#[async_trait]
impl Provider for LlamaCppProvider {
    async fn complete(&self, messages: Vec<Message>) -> Result<String> {
        let mut stream = self.generate_streaming(messages).await?;
        let mut full_response = String::new();

        while let Some(chunk) = tokio_stream::StreamExt::next(&mut stream).await {
            match chunk {
                Ok(response) => {
                    full_response.push_str(&response.content);
                    if response.done {
                        break;
                    }
                }
                Err(e) => return Err(e),
            }
        }

        Ok(full_response)
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let stream = self.generate_streaming(messages).await?;
        Ok(Box::new(stream))
    }

    fn name(&self) -> &str {
        &self.name
    }
}
