#![allow(clippy::useless_format)]

use anyhow::{Result, anyhow};
use candle_core::{Tensor, Device};
use crate::candle_transformer_layer::TransformerLayer;

/// Main implementation of the Qwen3 model using Candle
pub struct CandleQwen3Model {
    pub(crate) is_loaded: bool,
    
    pub(crate) hidden_dim: usize,
    pub(crate) num_layers: usize,
    pub(crate) num_heads: usize,
    pub(crate) head_dim: usize,
    pub(crate) vocab_size: usize,
    
    pub(crate) temperature: f32,
    
    pub(crate) device: Device,
    
    pub(crate) embedding: Tensor,
    pub(crate) position_embedding: Tensor,
    pub(crate) layers: Vec<TransformerLayer>,
    pub(crate) final_norm_weight: Tensor,
    pub(crate) final_norm_bias: Option<Tensor>,
    pub(crate) lm_head: Tensor,
}

impl CandleQwen3Model {
    /// Checks if the model is using GPU acceleration
    pub fn is_using_gpu(&self) -> bool {
        matches!(self.device, Device::Cuda(_) | Device::Metal(_))
    }
    
    /// Returns the name of the hardware acceleration being used
    pub fn get_acceleration_name(&self) -> &'static str {
        match self.device {
            Device::Cuda(_) => "CUDA (NVIDIA GPU)",
            Device::Metal(_) => "Metal (Apple GPU)",
            _ => "CPU",
        }
    }
    
    /// Placeholder for future implementation if needed
    /// Current generate method is in candle/generation.rs
    fn _generate_placeholder(&self, _input_tokens: &[u32], _max_tokens: usize) -> Result<Vec<u32>> {
        if !self.is_loaded {
            return Err(anyhow!("Model not loaded"));
        }
        
        Ok(Vec::new())
    }
}