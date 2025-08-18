use crate::{Error, Message, Provider, Result, Role, StreamResponse};
use arkavo_llama_cpp::{
    ffi, LlamaContext, LlamaModel, LlamaSampler, apply_chat_template, 
    tokenize_with_model, token_to_piece, batch_init_with_tokens, batch_free, decode_batch, 
    create_sampler_chain, test_minimal_init, init_llama_logging
};
use async_trait::async_trait;
use std::ffi::CString;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};
use tokio::{select, task::JoinHandle};
use tokio_stream::Stream;
use tokio_util::sync::CancellationToken;
use scopeguard::defer;

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
            max_tokens: 512,
            seed: 42,
        }
    }
}

/// AbortOnDropReceiverStream: Cancels producer task and frees resources on drop
pub struct AbortOnDropReceiverStream<T> {
    rx: mpsc::Receiver<Result<T, Error>>,
    handle: JoinHandle<()>,
    cancel: CancellationToken,
}

impl<T> Stream for AbortOnDropReceiverStream<T> {
    type Item = Result<T, Error>;
    
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let me = self.get_mut();
        Pin::new(&mut me.rx).poll_recv(cx)
    }
}

impl<T> Drop for AbortOnDropReceiverStream<T> {
    fn drop(&mut self) {
        // 1) Signal cancellation to producer
        self.cancel.cancel();
        // 2) Close receiver so producer sees send errors  
        self.rx.close();
        // 3) Abort producer task (in case it's stuck)
        self.handle.abort();
        // llama resources freed by producer's scopeguards
    }
}

fn make_stream<T>(
    handle: JoinHandle<()>,
    cancel: CancellationToken,
    rx: mpsc::Receiver<Result<T, Error>>,
) -> AbortOnDropReceiverStream<T> {
    AbortOnDropReceiverStream { rx, handle, cancel }
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

    pub fn new_with_config(model_name: String, model_path: String, config: SamplingConfig) -> Result<Self> {
        // Initialize debug flag from environment variable once
        if std::env::var("ARKAVO_DEBUG_CHAT").unwrap_or_default() == "1" {
            DEBUG_LLAMACPP.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        
        // Initialize llama.cpp logging (will check ARKAVO_DEBUG_CHAT internally)
        init_llama_logging();
        
        // Initialize backend with proper cleanup
        unsafe { ffi::llama_backend_init(); }
        
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

    /// Convert messages to llama chat format (static to avoid Send issues)
    fn messages_to_llama_chat_static(messages: &[Message]) -> Result<(Vec<ffi::llama_chat_message>, Vec<CString>)> {
        let mut llama_messages = Vec::new();
        let mut all_cstrings = Vec::new();

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

            all_cstrings.push(role_cstring);
            all_cstrings.push(content_cstring);
        }

        Ok((llama_messages, all_cstrings))
    }

    /// "Llama way" batching: single batch for entire prompt or proper chunking
    async fn decode_prompt_properly(
        ctx: &LlamaContext,
        tokens: &[ffi::llama_token],
        chunk_threshold: usize
    ) -> Result<()> {
        if tokens.len() <= chunk_threshold {
            // Single batch decode - preferred approach
            if DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed) {
                eprintln!("🚀 Single batch decode for {} tokens", tokens.len());
            }
            let mut batch = batch_init_with_tokens(tokens, 0, true);
            defer! { batch_free(&mut batch); }
            
            decode_batch(ctx, batch)
                .map_err(|e| Error::Config(format!("Failed to decode prompt batch: {}", e)))?;
        } else {
            // Chunked processing for very long prompts
            if DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed) {
                eprintln!("📦 Chunked processing for {} tokens", tokens.len());
            }
            let chunk_size = chunk_threshold;
            let mut pos_offset = 0i32;
            
            for (i, chunk) in tokens.chunks(chunk_size).enumerate() {
                let is_last_chunk = (i + 1) * chunk_size >= tokens.len();
                if DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed) {
                    eprintln!("  📦 Chunk {} with {} tokens (pos: {})", i + 1, chunk.len(), pos_offset);
                }
                
                let mut batch = batch_init_with_tokens(chunk, pos_offset, is_last_chunk);
                defer! { batch_free(&mut batch); }
                
                decode_batch(ctx, batch)
                    .map_err(|e| Error::Config(format!("Failed to decode chunk {}: {}", i + 1, e)))?;
                    
                pos_offset += chunk.len() as i32;
            }
        }
        Ok(())
    }

    async fn generate_streaming(&self, messages: Vec<Message>) -> Result<AbortOnDropReceiverStream<StreamResponse>> {
        // Prepare data outside of spawn to avoid Send issues
        let (llama_messages, _cstrings) = Self::messages_to_llama_chat_static(&messages)?;
        
        // Apply chat template
        let prompt_bytes = apply_chat_template(&llama_messages, true)
            .map_err(|e| Error::Config(format!("Failed to apply chat template: {}", e)))?;

        let (tx, rx) = mpsc::channel::<Result<StreamResponse, Error>>(64); // Bounded channel
        let cancel = CancellationToken::new();
        
        let model = self.model.clone();
        let context = self.context.clone();
        let config = self.config.clone();
        let cancel_clone = cancel.clone();

        let handle = tokio::spawn(async move {
            // Proper resource cleanup with scopeguards
            defer! { 
                unsafe { ffi::llama_backend_free(); }
                eprintln!("🧹 Cleaned up llama backend");
            }

            let start_time = Instant::now();
            let mut first_token_time: Option<Instant> = None;
            let mut tokens_generated = 0u32;

            let result = async {
                let ctx = context.lock().await;
                
                // Get vocab and tokenize inside the lock to avoid Send issues
                let vocab = model.get_vocab();
                let input_tokens = tokenize_with_model(vocab, &prompt_bytes)
                    .map_err(|e| Error::Config(format!("Failed to tokenize: {}", e)))?;

                tracing::info!("Input tokenized to {} tokens", input_tokens.len());
                if DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed) {
                    eprintln!("🔍 Input tokens: {} total", input_tokens.len());
                }

                // Create sampler with proper fallback
                let sampler = create_sampler_chain(
                    config.temperature, 
                    config.top_p, 
                    config.top_k, 
                    config.seed
                ).map_err(|e| Error::Config(format!("Failed to create sampler: {}", e)))?;

                // Get EOS token from the model's vocabulary
                let eos_token = model.get_eos_token();
                if DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed) {
                    eprintln!("EOS token from model: {}", eos_token);
                }
                
                // Decode prompt using proper "llama way" batching
                Self::decode_prompt_properly(&ctx, &input_tokens, 64).await?;
                if DEBUG_LLAMACPP.load(std::sync::atomic::Ordering::Relaxed) {
                    eprintln!("✅ Prompt decoded successfully");
                }

                // Generation loop with proper invariant: add token -> decode -> sample -> accept
                let max_generation = std::cmp::min(config.max_tokens, 1500);
                for i in 0..max_generation {
                    // Check for cancellation
                    select! {
                        _ = cancel_clone.cancelled() => {
                            eprintln!("🛑 Generation cancelled");
                            break;
                        }
                        default => {}
                    }

                    // Guardrail: assert logits exist before sampling
                    let logits_ptr = ctx.get_logits_ith(-1);
                    if logits_ptr.is_null() {
                        return Err(Error::Config("No logits available for sampling".to_string()));
                    }
                    
                    // Sample next token
                    let token = sampler.sample(&ctx, -1);
                    sampler.accept(token);
                    
                    // Check for EOS
                    if token == eos_token {
                        tracing::info!("Generation stopped at EOS token");
                        break;
                    }

                    // Convert token to text
                    let piece = token_to_piece(vocab, token, false)
                        .map_err(|e| Error::Config(format!("Failed to decode token: {}", e)))?;

                    // Record timing
                    if first_token_time.is_none() {
                        first_token_time = Some(Instant::now());
                    }
                    tokens_generated += 1;

                    // Send the token (non-blocking in async context)
                    let response = StreamResponse {
                        content: piece,
                        done: i + 1 >= config.max_tokens,
                    };

                    if tx.send(Ok(response)).await.is_err() {
                        break; // Receiver dropped
                    }

                    // Check context limit
                    let total_tokens = input_tokens.len() + tokens_generated as usize;
                    if total_tokens >= 1900 {
                        tracing::info!("Generation stopped at context limit");
                        break;
                    }

                    // Decode next token with proper position
                    let next_pos = input_tokens.len() as i32 + i as i32;
                    let mut next_batch = batch_init_with_tokens(&[token], next_pos, true);
                    defer! { batch_free(&mut next_batch); }
                    
                    if let Err(e) = decode_batch(&ctx, next_batch) {
                        let _ = tx.send(Err(Error::Config(format!("Failed to decode token: {}", e)))).await;
                        break;
                    }

                    // Yield periodically to prevent blocking
                    if i % 10 == 0 {
                        tokio::task::yield_now().await;
                    }
                }

                Ok::<(), Error>(())
            }.await;

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

                    let metrics = if let Some(ttft) = ttft {
                        format!(
                            "Generated {} tokens in {:.2}s ({:.1} tok/s, TTFT: {}ms)",
                            tokens_generated,
                            total_time.as_secs_f64(),
                            tokens_per_sec,
                            ttft.as_millis()
                        )
                    } else {
                        format!(
                            "Generated {} tokens in {:.2}s ({:.1} tok/s)",
                            tokens_generated,
                            total_time.as_secs_f64(),
                            tokens_per_sec
                        )
                    };

                    tracing::info!("{}", metrics);
                    eprintln!("📊 {}", metrics);

                    // Send final done response
                    let _ = tx.send(Ok(StreamResponse {
                        content: String::new(),
                        done: true,
                    })).await;
                }
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                }
            }
        });

        Ok(make_stream(handle, cancel, rx))
    }
}

#[async_trait]
impl Provider for LlamaCppProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn complete(&self, messages: Vec<Message>) -> Result<String> {
        let mut stream = self.stream(messages).await?;
        let mut result = String::new();
        
        while let Some(chunk) = tokio_stream::StreamExt::next(&mut stream).await {
            match chunk {
                Ok(response) => {
                    result.push_str(&response.content);
                    if response.done {
                        break;
                    }
                }
                Err(e) => return Err(e),
            }
        }
        
        Ok(result)
    }

    async fn stream(&self, messages: Vec<Message>) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let stream = self.generate_streaming(messages).await?;
        Ok(Box::new(stream))
    }
}