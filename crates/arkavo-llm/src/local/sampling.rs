use crate::Result;
use candle_core::Tensor;

pub struct SamplingParams {
    pub temperature: f64,
    pub top_p: f64,
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
