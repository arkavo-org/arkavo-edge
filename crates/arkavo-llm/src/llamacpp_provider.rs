use crate::{Error, Message, Provider, Result, Role, StreamResponse};
use arkavo_llama_cpp::{ffi, LlamaContext, LlamaModel, apply_chat_template, tokenize_with_model, detokenize};
use async_trait::async_trait;
use std::ffi::CString;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_stream::Stream;

pub struct LlamaCppProvider {
    model: Arc<LlamaModel>,
    context: Arc<Mutex<LlamaContext>>,
    name: String,
}

impl LlamaCppProvider {
    pub fn new(model_name: String, model_path: String) -> Result<Self> {
        let model = LlamaModel::from_file(&model_path)
            .map_err(|e| Error::Config(format!("Failed to load model: {}", e)))?;
        
        let context = LlamaContext::new(&model)
            .map_err(|e| Error::Config(format!("Failed to create context: {}", e)))?;

        Ok(Self {
            model: Arc::new(model),
            context: Arc::new(Mutex::new(context)),
            name: model_name,
        })
    }

    fn messages_to_llama_chat(&self, messages: &[Message]) -> Result<(Vec<ffi::llama_chat_message>, Vec<CString>)> {
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

    async fn generate_response(&self, messages: Vec<Message>) -> Result<String> {
        let (llama_messages, _cstrings) = self.messages_to_llama_chat(&messages)?;
        
        // Apply chat template
        let prompt_bytes = apply_chat_template(&llama_messages, true)
            .map_err(|e| Error::Config(format!("Failed to apply chat template: {}", e)))?;

        // Tokenize
        let vocab = self.model.get_vocab();
        let tokens = tokenize_with_model(vocab, &prompt_bytes)
            .map_err(|e| Error::Config(format!("Failed to tokenize: {}", e)))?;

        tracing::info!("Tokenized {} tokens", tokens.len());

        // For now, return a placeholder response 
        // TODO: Implement actual generation loop
        Ok("Hello! This is a placeholder response from llama.cpp.".to_string())
    }
}

#[async_trait]
impl Provider for LlamaCppProvider {
    async fn complete(&self, messages: Vec<Message>) -> Result<String> {
        self.generate_response(messages).await
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        // For now, just return the complete response as a single stream item
        // TODO: Implement proper streaming
        let response = self.generate_response(messages).await?;
        
        let stream = tokio_stream::iter(vec![
            Ok(StreamResponse {
                content: response,
                done: true,
            })
        ]);
        
        Ok(Box::new(stream))
    }

    fn name(&self) -> &str {
        &self.name
    }
}