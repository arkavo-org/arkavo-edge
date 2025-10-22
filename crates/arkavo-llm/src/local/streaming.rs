use crate::{Error, Result, StreamResponse};
use candle_core::{Device, Tensor};
use candle_transformers::models::quantized_llama::ModelWeights;
use std::sync::Arc;
use tokenizers::Tokenizer;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;

/// Create a streaming response for quantized models
pub fn create_quantized_stream(
    mut model: ModelWeights,
    tokenizer: Arc<Tokenizer>,
    prompt_ids: Vec<i64>,
    eos_token_id: i64,
    max_tokens: usize,
    device: Device,
) -> impl Stream<Item = Result<StreamResponse>> + Send + Unpin {
    let (tx, rx) = mpsc::channel::<Result<StreamResponse>>(32);

    tokio::spawn(async move {
        let mut ids = prompt_ids.clone();
        let _start_len = ids.len();

        for _i in 0..max_tokens {
            // Create input tensor
            let input = match Tensor::new(ids.as_slice(), &device).and_then(|t| t.unsqueeze(0)) {
                Ok(t) => t,
                Err(e) => {
                    let _ = tx
                        .send(Err(Error::Model(format!("Failed to create tensor: {e}"))))
                        .await;
                    return;
                }
            };

            // Forward pass
            let logits = match model.forward(&input, ids.len() - 1) {
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

            // Decode only the new token
            match tokenizer.decode(&[next as u32], false) {
                Ok(text) => {
                    // Send streaming response
                    let response = StreamResponse {
                        content: text,
                        done: false,
                    };

                    if tx.send(Ok(response)).await.is_err() {
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

        // Send final response
        let _ = tx
            .send(Ok(StreamResponse {
                content: String::new(),
                done: true,
            }))
            .await;
    });

    ReceiverStream::new(rx)
}

/// Helper to create a box stream from quantized model
pub fn stream_quantized_response(
    model: ModelWeights,
    tokenizer: Arc<Tokenizer>,
    prompt_ids: Vec<i64>,
    eos_token_id: i64,
    max_tokens: usize,
    device: Device,
) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
    let stream = create_quantized_stream(
        model,
        tokenizer,
        prompt_ids,
        eos_token_id,
        max_tokens,
        device,
    );

    Ok(Box::new(stream))
}
