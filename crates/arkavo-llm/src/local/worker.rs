use crate::{Error, Result};
use candle_core::{Device, Tensor};
use candle_transformers::models::llama::{Cache, Config as LlamaConfig, Llama};
use std::sync::Arc;
use tokenizers::Tokenizer;
use tokio::sync::{mpsc, oneshot};

/// Request types for the worker task
pub enum WorkerRequest {
    /// Generate tokens from a prompt
    Generate {
        prompt_ids: Vec<i64>,
        max_tokens: usize,
        eos_token_id: i64,
        response: oneshot::Sender<Result<Vec<i64>>>,
    },
    /// Shutdown the worker
    Shutdown,
}

/// Worker task that owns the model and cache
pub struct ModelWorker {
    model: Llama,
    cache: Cache,
    device: Device,
    config: LlamaConfig,
}

impl ModelWorker {
    /// Create a new model worker
    pub fn new(model: Llama, config: LlamaConfig, device: Device) -> Self {
        let cache = Cache::new(true, candle_core::DType::F32, &config, &device)
            .expect("Failed to create cache");

        Self {
            model,
            cache,
            device,
            config,
        }
    }

    /// Run the worker task
    pub async fn run(mut self, mut receiver: mpsc::Receiver<WorkerRequest>) {
        while let Some(request) = receiver.recv().await {
            match request {
                WorkerRequest::Generate {
                    prompt_ids,
                    max_tokens,
                    eos_token_id,
                    response,
                } => {
                    let result = self.generate(prompt_ids, max_tokens, eos_token_id).await;
                    let _ = response.send(result);
                }
                WorkerRequest::Shutdown => {
                    tracing::info!("Model worker shutting down");
                    break;
                }
            }
        }
    }

    /// Generate tokens
    async fn generate(
        &mut self,
        mut ids: Vec<i64>,
        max_tokens: usize,
        eos_token_id: i64,
    ) -> Result<Vec<i64>> {
        let start_len = ids.len();

        for index in 0..max_tokens {
            let position = start_len + index;

            // Create input tensor
            let input = if index == 0 {
                // First iteration: process entire prompt
                Tensor::new(ids.as_slice(), &self.device)
                    .map_err(|e| Error::Model(format!("Failed to create tensor: {e}")))?
                    .unsqueeze(0)
                    .map_err(|e| Error::Model(format!("Failed to unsqueeze: {e}")))?
            } else {
                // Subsequent iterations: process only the last token
                Tensor::new(&[*ids.last().unwrap()], &self.device)
                    .map_err(|e| Error::Model(format!("Failed to create tensor: {e}")))?
                    .unsqueeze(0)
                    .map_err(|e| Error::Model(format!("Failed to unsqueeze: {e}")))?
            };

            // Forward pass with cache
            let logits = self
                .model
                .forward(&input, position, &mut self.cache)
                .map_err(|e| Error::Model(format!("Forward pass failed: {e}")))?;

            // Get next token
            let next = logits
                .squeeze(0)
                .map_err(|e| Error::Model(format!("Failed to squeeze: {e}")))?
                .get(
                    logits
                        .dim(1)
                        .map_err(|e| Error::Model(format!("Failed to get dim: {e}")))?
                        - 1,
                )
                .map_err(|e| Error::Model(format!("Failed to get last logits: {e}")))?
                .argmax(0)
                .map_err(|e| Error::Model(format!("Failed to argmax: {e}")))?
                .to_scalar::<i64>()
                .map_err(|e| Error::Model(format!("Failed to get scalar: {e}")))?;

            ids.push(next);

            if next == eos_token_id {
                break;
            }
        }

        Ok(ids)
    }
}

/// Handle for communicating with the model worker
pub struct WorkerHandle {
    sender: mpsc::Sender<WorkerRequest>,
}

impl WorkerHandle {
    /// Create a new worker handle and spawn the worker task
    pub fn spawn(model: Llama, config: LlamaConfig, device: Device) -> Self {
        let (sender, receiver) = mpsc::channel(32);
        let worker = ModelWorker::new(model, config, device);

        // Spawn the worker task
        tokio::spawn(async move {
            worker.run(receiver).await;
        });

        Self { sender }
    }

    /// Generate tokens from a prompt
    pub async fn generate(
        &self,
        prompt_ids: Vec<i64>,
        max_tokens: usize,
        eos_token_id: i64,
    ) -> Result<Vec<i64>> {
        let (response_tx, response_rx) = oneshot::channel();

        self.sender
            .send(WorkerRequest::Generate {
                prompt_ids,
                max_tokens,
                eos_token_id,
                response: response_tx,
            })
            .await
            .map_err(|_| Error::Model("Worker task has stopped".to_string()))?;

        response_rx
            .await
            .map_err(|_| Error::Model("Worker task failed to respond".to_string()))?
    }

    /// Shutdown the worker
    pub async fn shutdown(&self) -> Result<()> {
        self.sender
            .send(WorkerRequest::Shutdown)
            .await
            .map_err(|_| Error::Model("Failed to send shutdown request".to_string()))?;
        Ok(())
    }
}

/// Worker for streaming generation
pub struct StreamingWorker {
    model: Llama,
    cache: Cache,
    device: Device,
    config: LlamaConfig,
}

impl StreamingWorker {
    /// Create a new streaming worker
    pub fn new(model: Llama, config: LlamaConfig, device: Device) -> Self {
        let cache = Cache::new(true, candle_core::DType::F32, &config, &device)
            .expect("Failed to create cache");

        Self {
            model,
            cache,
            device,
            config,
        }
    }

    /// Generate tokens with streaming
    pub async fn generate_stream(
        mut self,
        prompt_ids: Vec<i64>,
        max_tokens: usize,
        eos_token_id: i64,
        tokenizer: Arc<Tokenizer>,
    ) -> Result<mpsc::Receiver<Result<String>>> {
        let (tx, rx) = mpsc::channel(32);
        let mut ids = prompt_ids.clone();
        let start_len = ids.len();

        // Spawn task to generate tokens
        tokio::spawn(async move {
            for index in 0..max_tokens {
                let position = start_len + index;

                // Create input tensor
                let input = if index == 0 {
                    // First iteration: process entire prompt
                    match Tensor::new(ids.as_slice(), &self.device).and_then(|t| t.unsqueeze(0)) {
                        Ok(t) => t,
                        Err(e) => {
                            let _ = tx
                                .send(Err(Error::Model(format!("Failed to create tensor: {e}"))))
                                .await;
                            return;
                        }
                    }
                } else {
                    // Subsequent iterations: process only the last token
                    match Tensor::new(&[*ids.last().unwrap()], &self.device)
                        .and_then(|t| t.unsqueeze(0))
                    {
                        Ok(t) => t,
                        Err(e) => {
                            let _ = tx
                                .send(Err(Error::Model(format!("Failed to create tensor: {e}"))))
                                .await;
                            return;
                        }
                    }
                };

                // Forward pass with cache
                let logits = match self.model.forward(&input, position, &mut self.cache) {
                    Ok(l) => l,
                    Err(e) => {
                        let _ = tx
                            .send(Err(Error::Model(format!("Forward pass failed: {e}"))))
                            .await;
                        return;
                    }
                };

                // Get next token
                let next = match logits
                    .squeeze(0)
                    .and_then(|l| l.get(logits.dim(1).unwrap_or(0).saturating_sub(1)))
                    .and_then(|l| l.argmax(0))
                    .and_then(|l| l.to_scalar::<i64>())
                {
                    Ok(n) => n,
                    Err(e) => {
                        let _ = tx
                            .send(Err(Error::Model(format!("Failed to get next token: {e}"))))
                            .await;
                        return;
                    }
                };

                ids.push(next);

                // Decode the new token
                match tokenizer.decode(&[next as u32], false) {
                    Ok(text) => {
                        if tx.send(Ok(text)).await.is_err() {
                            // Receiver dropped
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(Error::Model(format!("Failed to decode token: {e}"))))
                            .await;
                        return;
                    }
                }

                if next == eos_token_id {
                    break;
                }
            }
        });

        Ok(rx)
    }
}
