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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_metrics_50_percent_reduction() {
        let m = CompressionMetrics::new(1000, 500, 10);
        assert_eq!(m.original_tokens, 1000);
        assert_eq!(m.compressed_tokens, 500);
        assert!((m.reduction_percent - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compression_metrics_zero_original() {
        let m = CompressionMetrics::new(0, 0, 5);
        assert!((m.reduction_percent - 0.0).abs() < f64::EPSILON);
        assert!((m.cost_saved - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compression_metrics_cost_savings() {
        let m = CompressionMetrics::new(1_000_000, 500_000, 20);
        let expected_cost = (500_000.0 / 1_000_000.0) * 0.30;
        assert!((m.cost_saved - expected_cost).abs() < 1e-9);
    }

    #[test]
    fn test_compression_metrics_with_quality() {
        let m = CompressionMetrics::new(100, 50, 1).with_quality(0.85);
        assert!((m.information_retention - 0.85).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compression_metrics_default_retention() {
        let m = CompressionMetrics::new(100, 50, 1);
        assert!((m.information_retention - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compression_stats_new_all_zeros() {
        let s = CompressionStats::new();
        assert_eq!(s.total_compressions, 0);
        assert_eq!(s.total_tokens_saved, 0);
        assert!((s.total_cost_saved - 0.0).abs() < f64::EPSILON);
        assert!((s.average_reduction - 0.0).abs() < f64::EPSILON);
        assert!((s.average_quality - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compression_stats_default_same_as_new() {
        let new_stats = CompressionStats::new();
        let default_stats = CompressionStats::default();
        assert_eq!(
            new_stats.total_compressions,
            default_stats.total_compressions
        );
        assert_eq!(
            new_stats.total_tokens_saved,
            default_stats.total_tokens_saved
        );
        assert!((new_stats.total_cost_saved - default_stats.total_cost_saved).abs() < f64::EPSILON);
        assert!(
            (new_stats.average_reduction - default_stats.average_reduction).abs() < f64::EPSILON
        );
        assert!((new_stats.average_quality - default_stats.average_quality).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compression_stats_record_single() {
        let mut s = CompressionStats::new();
        let m = CompressionMetrics::new(1000, 500, 10);
        s.record(&m);
        assert_eq!(s.total_compressions, 1);
        assert_eq!(s.total_tokens_saved, 500);
        assert!((s.average_reduction - 50.0).abs() < f64::EPSILON);
        assert!((s.average_quality - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compression_stats_record_two_moving_average() {
        let mut s = CompressionStats::new();
        let m1 = CompressionMetrics::new(1000, 500, 10); // 50% reduction
        let m2 = CompressionMetrics::new(1000, 0, 10); // 100% reduction
        s.record(&m1);
        s.record(&m2);
        assert_eq!(s.total_compressions, 2);
        assert!((s.average_reduction - 75.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compression_stats_cumulative_cost() {
        let mut s = CompressionStats::new();
        let m1 = CompressionMetrics::new(1_000_000, 500_000, 10);
        let m2 = CompressionMetrics::new(1_000_000, 500_000, 10);
        s.record(&m1);
        s.record(&m2);
        let expected_total = m1.cost_saved + m2.cost_saved;
        assert!((s.total_cost_saved - expected_total).abs() < 1e-9);
    }
}
