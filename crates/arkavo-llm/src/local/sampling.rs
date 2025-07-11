use crate::Result;
use candle_core::Tensor;

/// Parameters for controlling text generation sampling behavior
#[derive(Debug, Clone)]
pub struct SamplingParams {
    /// Temperature for sampling. Higher values (e.g., 1.0) make the output more random,
    /// while lower values (e.g., 0.1) make it more deterministic. Range: (0.0, 2.0]
    pub temperature: f64,

    /// Top-p (nucleus) sampling threshold. Only tokens with cumulative probability
    /// up to this value are considered. Range: (0.0, 1.0]
    pub top_p: f64,

    /// Repetition penalty to reduce token repetition. Values > 1.0 penalize repeated tokens.
    /// Range: [1.0, 2.0]
    pub repetition_penalty: f64,
}

impl Default for SamplingParams {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            top_p: 0.9,
            repetition_penalty: 1.15,
        }
    }
}

/// Apply temperature scaling and sampling to logits
///
/// # Arguments
/// * `logits` - Raw model output logits (before softmax)
/// * `temperature` - Temperature for sampling (higher = more random)
/// * `top_p` - Nucleus sampling threshold (0.0 to 1.0)
/// * `repetition_penalty` - Penalty for previously generated tokens
/// * `previous_tokens` - List of previously generated token IDs
///
/// # Returns
/// The sampled token ID
pub fn sample_next_token(
    logits: &Tensor,
    temperature: f64,
    top_p: f64,
    repetition_penalty: f64,
    previous_tokens: &[u32],
) -> Result<u32> {
    let device = logits.device();
    let vocab_size = logits.dims()[0];

    // Apply repetition penalty
    let mut logits = if (repetition_penalty - 1.0).abs() > f64::EPSILON
        && !previous_tokens.is_empty()
    {
        let mut logits_vec = logits
            .to_vec1::<f32>()
            .map_err(|e| crate::Error::Model(format!("Failed to convert logits to vec: {e}")))?;

        // Apply penalty to previously generated tokens
        for &token_id in previous_tokens.iter() {
            if (token_id as usize) < vocab_size {
                if logits_vec[token_id as usize] > 0.0 {
                    logits_vec[token_id as usize] /= repetition_penalty as f32;
                } else {
                    logits_vec[token_id as usize] *= repetition_penalty as f32;
                }
            }
        }

        Tensor::from_vec(logits_vec, vocab_size, device)
            .map_err(|e| crate::Error::Model(format!("Failed to create tensor from vec: {e}")))?
    } else {
        logits.clone()
    };

    // Apply temperature
    if (temperature - 1.0).abs() > f64::EPSILON {
        logits = (&logits / temperature)
            .map_err(|e| crate::Error::Model(format!("Failed to apply temperature: {e}")))?;
    }

    // Convert to probabilities with softmax
    let probs = candle_nn::ops::softmax(&logits, 0)
        .map_err(|e| crate::Error::Model(format!("Failed to apply softmax: {e}")))?;

    // Apply top-p (nucleus) sampling
    if top_p < 1.0 - f64::EPSILON {
        sample_top_p(&probs, top_p)
    } else {
        // Simple sampling from the distribution
        sample_from_probs(&probs)
    }
}

/// Sample from probability distribution using top-p (nucleus) sampling
fn sample_top_p(probs: &Tensor, top_p: f64) -> Result<u32> {
    let probs_vec = probs
        .to_vec1::<f32>()
        .map_err(|e| crate::Error::Model(format!("Failed to convert probs to vec: {e}")))?;

    // Create sorted indices
    let mut indices: Vec<usize> = (0..probs_vec.len()).collect();
    indices.sort_by(|&a, &b| probs_vec[b].partial_cmp(&probs_vec[a]).unwrap());

    // Compute cumulative probabilities
    let mut cumsum = 0.0;
    let mut cutoff = indices.len();

    for (i, &idx) in indices.iter().enumerate() {
        cumsum += probs_vec[idx] as f64;
        if cumsum > top_p {
            cutoff = i + 1;
            break;
        }
    }

    // Sample from the truncated distribution
    let truncated_indices = &indices[..cutoff];
    let truncated_probs: Vec<f32> = truncated_indices
        .iter()
        .map(|&idx| probs_vec[idx])
        .collect();

    // Normalize
    let sum: f32 = truncated_probs.iter().sum();
    let normalized_probs: Vec<f32> = truncated_probs.iter().map(|&p| p / sum).collect();

    // Sample
    let mut rng = rand::thread_rng();
    use rand::distributions::{Distribution, WeightedIndex};

    let dist = WeightedIndex::new(&normalized_probs)
        .map_err(|e| crate::Error::Model(format!("Failed to create weighted distribution: {e}")))?;

    Ok(truncated_indices[dist.sample(&mut rng)] as u32)
}

/// Simple sampling from probability distribution
fn sample_from_probs(probs: &Tensor) -> Result<u32> {
    let probs_vec = probs
        .to_vec1::<f32>()
        .map_err(|e| crate::Error::Model(format!("Failed to convert probs to vec: {e}")))?;

    let mut rng = rand::thread_rng();
    use rand::distributions::{Distribution, WeightedIndex};

    let dist = WeightedIndex::new(&probs_vec)
        .map_err(|e| crate::Error::Model(format!("Failed to create weighted distribution: {e}")))?;

    Ok(dist.sample(&mut rng) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::{Device, Tensor};

    #[test]
    fn test_sampling_params_default() {
        let params = SamplingParams::default();
        assert!((params.temperature - 0.7).abs() < f64::EPSILON);
        assert!((params.top_p - 0.9).abs() < f64::EPSILON);
        assert!((params.repetition_penalty - 1.15).abs() < f64::EPSILON);
    }

    #[test]
    fn test_sample_from_probs() -> Result<()> {
        let device = Device::Cpu;

        // Create a simple probability distribution
        let probs = Tensor::new(&[0.1f32, 0.2, 0.3, 0.4], &device)?;

        // Sample multiple times to ensure it works
        for _ in 0..10 {
            let token_id = sample_from_probs(&probs)?;
            assert!(token_id < 4, "Token ID should be within vocabulary size");
        }

        Ok(())
    }

    #[test]
    fn test_sample_top_p() -> Result<()> {
        let device = Device::Cpu;

        // Create a probability distribution where one token dominates
        let probs = Tensor::new(&[0.8f32, 0.1, 0.05, 0.05], &device)?;

        // With low top_p, should mostly sample the dominant token
        let mut dominant_count = 0;
        for _ in 0..100 {
            let token_id = sample_top_p(&probs, 0.85)?;
            if token_id == 0 {
                dominant_count += 1;
            }
        }

        // Should sample the dominant token most of the time
        assert!(
            dominant_count > 70,
            "Should sample dominant token frequently with low top_p"
        );

        Ok(())
    }

    #[test]
    fn test_repetition_penalty() -> Result<()> {
        let device = Device::Cpu;
        let vocab_size = 5;

        // Create uniform logits
        let logits = Tensor::new(&[1.0f32, 1.0, 1.0, 1.0, 1.0], &device)?;

        // Apply sampling with repetition penalty
        let previous_tokens = vec![0, 1]; // Tokens that should be penalized
        let token_id = sample_next_token(&logits, 1.0, 1.0, 1.5, &previous_tokens)?;

        // The selected token should be valid
        assert!(
            token_id < vocab_size as u32,
            "Token ID should be within vocabulary"
        );

        Ok(())
    }

    #[test]
    fn test_temperature_scaling() -> Result<()> {
        let device = Device::Cpu;

        // Create logits with clear preference
        let logits = Tensor::new(&[5.0f32, 1.0, 1.0, 1.0], &device)?;

        // Sample with low temperature (more deterministic)
        let mut high_prob_count = 0;
        for _ in 0..100 {
            let token_id = sample_next_token(&logits, 0.1, 1.0, 1.0, &[])?;
            if token_id == 0 {
                high_prob_count += 1;
            }
        }

        // With low temperature, should almost always pick the highest logit
        assert!(
            high_prob_count > 95,
            "Low temperature should make sampling more deterministic"
        );

        Ok(())
    }
}
