use anyhow::{Result, anyhow};
use candle_core::Tensor;

pub struct KVCache {
    pub(crate) layers: Vec<(Tensor, Tensor)>,
}

impl KVCache {
    pub fn new(
        num_layers: usize,
        num_heads: usize,
        seq_len: usize,
        head_dim: usize,
        device: &candle_core::Device,
    ) -> Result<Self> {
        let mut layers = Vec::with_capacity(num_layers);
        
        for _ in 0..num_layers {
            let key_cache = Tensor::zeros((1, num_heads, seq_len, head_dim), candle_core::DType::F32, device)?;
            let value_cache = Tensor::zeros((1, num_heads, seq_len, head_dim), candle_core::DType::F32, device)?;
            
            layers.push((key_cache, value_cache));
        }
        
        Ok(Self { layers })
    }
    
    pub fn update_cache(&mut self, layer_idx: usize, position: usize, key: &Tensor, value: &Tensor) -> Result<()> {
        if layer_idx >= self.layers.len() {
            return Err(anyhow!("Invalid layer index: {}", layer_idx));
        }
        
        let (k_cache, v_cache) = &mut self.layers[layer_idx];
        
        let cache_seq_len = k_cache.shape().dims()[2];
        if position >= cache_seq_len {
            return Ok(());
        }
        
        let key_dims = key.shape().dims();
        let value_dims = value.shape().dims();
        
        if key_dims.len() == 4 && value_dims.len() == 4 {
            k_cache.slice_assign(&[0..1, 0..key.dim(1)?, position..position+1, 0..key.dim(3)?], key)?;
            v_cache.slice_assign(&[0..1, 0..value.dim(1)?, position..position+1, 0..value.dim(3)?], value)?;
        } else {
            let adapted_key = if key_dims != v_cache.shape().dims() {
                let k_shape = k_cache.shape().dims();
                let key_batch = k_shape[0];
                let key_heads = k_shape[1];
                let key_head_dim = k_shape[3];
                
                let key_3d = if key_dims.len() < 3 {
                    key.reshape((key_batch, key_heads, key_head_dim))?
                } else {
                    key.clone()
                };
                
                if key_3d.shape().dims().len() < 4 {
                    key_3d.unsqueeze(2)?
                } else {
                    key_3d
                }
            } else {
                key.clone()
            };
            
            let adapted_value = if value_dims != v_cache.shape().dims() {
                let v_shape = v_cache.shape().dims();
                let value_batch = v_shape[0];
                let value_heads = v_shape[1];
                let value_head_dim = v_shape[3];
                
                let value_3d = if value_dims.len() < 3 {
                    value.reshape((value_batch, value_heads, value_head_dim))?
                } else {
                    value.clone()
                };
                
                if value_3d.shape().dims().len() < 4 {
                    value_3d.unsqueeze(2)?
                } else {
                    value_3d
                }
            } else {
                value.clone()
            };
            
            k_cache.slice_assign(&[0..1, 0..adapted_key.dim(1)?, position..position+1, 0..adapted_key.dim(3)?], &adapted_key)?;
            v_cache.slice_assign(&[0..1, 0..adapted_value.dim(1)?, position..position+1, 0..adapted_value.dim(3)?], &adapted_value)?;
        }
        
        Ok(())
    }
    
    pub fn get_cache_for_layer(&self, layer_idx: usize, position: usize) -> Result<(Tensor, Tensor)> {
        if layer_idx >= self.layers.len() {
            return Err(anyhow!("Invalid layer index: {}", layer_idx));
        }
        
        let (k_cache, v_cache) = &self.layers[layer_idx];
        
        let seq_len = k_cache.shape().dims()[2];
        let actual_pos = std::cmp::min(position + 1, seq_len);
        
        let k_cache_slice = k_cache.narrow(2, 0, actual_pos)?;
        let v_cache_slice = v_cache.narrow(2, 0, actual_pos)?;
        
        Ok((k_cache_slice, v_cache_slice))
    }
}