use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionMetrics {
    pub original_tokens: u32,
    pub compressed_tokens: u32,
    pub reduction_percent: f64,
    pub compression_time_ms: u64,
    pub information_retention: f64,
    pub cost_saved: f64,
}

impl CompressionMetrics {
    pub fn new(original_tokens: u32, compressed_tokens: u32, compression_time_ms: u64) -> Self {
        let reduction_percent = if original_tokens > 0 {
            ((original_tokens - compressed_tokens) as f64 / original_tokens as f64) * 100.0
        } else {
            0.0
        };

        let cost_per_million_input = 0.30;
        let cost_saved =
            ((original_tokens - compressed_tokens) as f64 / 1_000_000.0) * cost_per_million_input;

        Self {
            original_tokens,
            compressed_tokens,
            reduction_percent,
            compression_time_ms,
            information_retention: 1.0,
            cost_saved,
        }
    }

    pub fn with_quality(mut self, information_retention: f64) -> Self {
        self.information_retention = information_retention;
        self
    }
}

#[derive(Debug, Clone)]
pub struct CompressionStats {
    pub total_compressions: u64,
    pub total_tokens_saved: u64,
    pub total_cost_saved: f64,
    pub average_reduction: f64,
    pub average_quality: f64,
}

impl CompressionStats {
    pub fn new() -> Self {
        Self {
            total_compressions: 0,
            total_tokens_saved: 0,
            total_cost_saved: 0.0,
            average_reduction: 0.0,
            average_quality: 0.0,
        }
    }

    pub fn record(&mut self, metrics: &CompressionMetrics) {
        self.total_compressions += 1;
        self.total_tokens_saved += (metrics.original_tokens - metrics.compressed_tokens) as u64;
        self.total_cost_saved += metrics.cost_saved;

        let n = self.total_compressions as f64;
        self.average_reduction =
            ((self.average_reduction * (n - 1.0)) + metrics.reduction_percent) / n;
        self.average_quality =
            ((self.average_quality * (n - 1.0)) + metrics.information_retention) / n;
    }
}

impl Default for CompressionStats {
    fn default() -> Self {
        Self::new()
    }
}
