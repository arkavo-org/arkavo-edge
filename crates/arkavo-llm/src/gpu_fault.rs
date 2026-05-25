use std::time::{Duration, Instant};

/// Classification of GPU faults detected during inference
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuFaultKind {
    /// macOS Metal InnocentVictim / MTLCommandBuffer kill
    MetalKill,
    /// Generic llama_decode non-zero return (transient)
    DecodeFailure,
    /// Context became invalid after a prior fault
    ContextCorrupt,
}

impl std::fmt::Display for GpuFaultKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MetalKill => write!(f, "MetalKill"),
            Self::DecodeFailure => write!(f, "DecodeFailure"),
            Self::ContextCorrupt => write!(f, "ContextCorrupt"),
        }
    }
}

/// A recorded GPU fault event
#[derive(Debug, Clone)]
pub struct GpuFault {
    pub kind: GpuFaultKind,
    pub raw_message: String,
    pub timestamp: Instant,
    pub attempt: u32,
}

/// Classify a raw decode error string into a GPU fault kind.
///
/// Returns `None` if the error is not GPU-related (e.g., tokenization failure).
/// Metal kills typically produce error codes -3 or messages containing
/// "InnocentVictim", "MTLCommandBuffer", or "kIOGPUCommandBufferCallbackError".
pub fn classify_gpu_fault(error_msg: &str) -> Option<GpuFaultKind> {
    // Metal-specific kill signatures
    if error_msg.contains("InnocentVictim")
        || error_msg.contains("kIOGPUCommandBufferCallbackError")
        || error_msg.contains("MTLCommandBuffer")
        || error_msg.contains("GPU command buffer")
    {
        return Some(GpuFaultKind::MetalKill);
    }

    // llama_decode error code -3 is Metal GPU eviction
    if error_msg.contains("code: -3") {
        return Some(GpuFaultKind::MetalKill);
    }

    // llama_decode code -1 = "invalid input batch" (b9292 llama.h). This is a
    // programming/state error in batch construction (e.g. non-monotonic
    // positions in M-RoPE mode after a spec rollback), not a GPU issue.
    // Retrying with `reset_gpu_status()` will produce the exact same -1 and
    // burns time pretending it could recover. Return None so the caller
    // propagates it as a regular Error::Config and aborts instead.
    if error_msg.contains("code: -1") {
        return None;
    }

    // Any other llama_decode failure is a generic decode error
    if error_msg.contains("llama_decode failed") {
        return Some(GpuFaultKind::DecodeFailure);
    }

    None
}

/// Time-windowed circuit breaker for GPU faults.
///
/// Tracks fault timestamps within a sliding window. When the count reaches
/// the threshold, the breaker trips and signals CPU-only fallback.
pub struct GpuCircuitBreaker {
    faults: Vec<Instant>,
    window: Duration,
    threshold: u32,
    tripped: bool,
    max_retries: u32,
}

impl Default for GpuCircuitBreaker {
    fn default() -> Self {
        Self::new(3, Duration::from_mins(1), 3)
    }
}

impl GpuCircuitBreaker {
    pub fn new(threshold: u32, window: Duration, max_retries: u32) -> Self {
        Self {
            faults: Vec::new(),
            window,
            threshold,
            tripped: false,
            max_retries,
        }
    }

    /// Record a GPU fault. Returns `true` if the breaker just tripped.
    pub fn record_fault(&mut self) -> bool {
        let now = Instant::now();
        self.faults.push(now);
        self.prune_old(now);

        if !self.tripped && self.faults.len() >= self.threshold as usize {
            self.tripped = true;
            return true;
        }
        false
    }

    /// Whether the circuit breaker has tripped (CPU-only fallback active).
    pub fn is_tripped(&self) -> bool {
        self.tripped
    }

    /// Whether a retry should be attempted for the given attempt number.
    pub fn should_retry(&self, attempt: u32) -> bool {
        !self.tripped && attempt < self.max_retries
    }

    /// Exponential backoff duration: 100ms * 2^attempt.
    pub fn backoff_duration(&self, attempt: u32) -> Duration {
        Duration::from_millis(100 * 2u64.pow(attempt))
    }

    /// Reset the circuit breaker (e.g., after successful GPU inference).
    pub fn reset(&mut self) {
        self.faults.clear();
        self.tripped = false;
    }

    /// Remove faults older than the window from the current timestamp.
    fn prune_old(&mut self, now: Instant) {
        let cutoff = now.checked_sub(self.window).unwrap_or(now);
        self.faults.retain(|t| *t >= cutoff);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_innocent_victim_error() {
        let msg = "llama_decode failed with code: -3";
        assert_eq!(classify_gpu_fault(msg), Some(GpuFaultKind::MetalKill));
    }

    #[test]
    fn test_classify_metal_command_buffer() {
        let msg = "MTLCommandBuffer execution failed: InnocentVictim";
        assert_eq!(classify_gpu_fault(msg), Some(GpuFaultKind::MetalKill));
    }

    #[test]
    fn test_classify_iokit_error() {
        let msg = "kIOGPUCommandBufferCallbackError during batch decode";
        assert_eq!(classify_gpu_fault(msg), Some(GpuFaultKind::MetalKill));
    }

    #[test]
    fn test_classify_gpu_command_buffer() {
        let msg = "GPU command buffer was killed by the system";
        assert_eq!(classify_gpu_fault(msg), Some(GpuFaultKind::MetalKill));
    }

    #[test]
    fn test_classify_invalid_batch_is_not_gpu_fault() {
        // llama.h: code -1 = "invalid input batch". It's a batch-construction
        // bug (e.g. spec rollback on an M-RoPE model), not a GPU issue. The
        // GPU-reset/retry path can't fix it and only hides the real error,
        // so the classifier must return None and let it propagate as a
        // regular Config error.
        let msg = "spec batch at pos 4110: llama_decode failed with code: -1";
        assert_eq!(classify_gpu_fault(msg), None);
    }

    #[test]
    fn test_classify_generic_decode_error() {
        // Other non-zero codes (-2, fatal errors, etc.) are still treated as
        // generic decode failures eligible for the GPU-reset retry path.
        let msg = "llama_decode failed with code: -2";
        assert_eq!(classify_gpu_fault(msg), Some(GpuFaultKind::DecodeFailure));
    }

    #[test]
    fn test_classify_non_gpu_error() {
        let msg = "Failed to tokenize: invalid utf-8";
        assert_eq!(classify_gpu_fault(msg), None);
    }

    #[test]
    fn test_classify_empty_string() {
        assert_eq!(classify_gpu_fault(""), None);
    }

    #[test]
    fn test_circuit_breaker_stays_closed_under_threshold() {
        let mut cb = GpuCircuitBreaker::default();
        cb.record_fault();
        cb.record_fault();
        assert!(!cb.is_tripped());
    }

    #[test]
    fn test_circuit_breaker_trips_at_threshold() {
        let mut cb = GpuCircuitBreaker::default();
        cb.record_fault();
        cb.record_fault();
        let tripped = cb.record_fault();
        assert!(tripped);
        assert!(cb.is_tripped());
    }

    #[test]
    fn test_circuit_breaker_window_expiry() {
        let mut cb = GpuCircuitBreaker::new(3, Duration::from_millis(100), 3);
        // Record 2 faults, wait for them to expire, then record 1 more
        cb.record_fault();
        cb.record_fault();
        // Simulate time passing by manipulating faults directly
        let old = Instant::now()
            .checked_sub(Duration::from_millis(200))
            .unwrap();
        cb.faults = vec![old, old];
        let tripped = cb.record_fault();
        assert!(!tripped, "old faults should have been pruned");
        assert!(!cb.is_tripped());
    }

    #[test]
    fn test_circuit_breaker_reset() {
        let mut cb = GpuCircuitBreaker::default();
        cb.record_fault();
        cb.record_fault();
        cb.record_fault();
        assert!(cb.is_tripped());
        cb.reset();
        assert!(!cb.is_tripped());
    }

    #[test]
    fn test_should_retry_respects_max() {
        let cb = GpuCircuitBreaker::default(); // max_retries=3
        assert!(cb.should_retry(0));
        assert!(cb.should_retry(1));
        assert!(cb.should_retry(2));
        assert!(!cb.should_retry(3));
    }

    #[test]
    fn test_should_retry_false_when_tripped() {
        let mut cb = GpuCircuitBreaker::default();
        cb.record_fault();
        cb.record_fault();
        cb.record_fault();
        assert!(!cb.should_retry(0));
    }

    #[test]
    fn test_backoff_exponential() {
        let cb = GpuCircuitBreaker::default();
        assert_eq!(cb.backoff_duration(0), Duration::from_millis(100));
        assert_eq!(cb.backoff_duration(1), Duration::from_millis(200));
        assert_eq!(cb.backoff_duration(2), Duration::from_millis(400));
    }

    #[test]
    fn test_gpu_fault_kind_display() {
        assert_eq!(GpuFaultKind::MetalKill.to_string(), "MetalKill");
        assert_eq!(GpuFaultKind::DecodeFailure.to_string(), "DecodeFailure");
        assert_eq!(GpuFaultKind::ContextCorrupt.to_string(), "ContextCorrupt");
    }

    #[test]
    fn test_gpu_fault_struct() {
        let fault = GpuFault {
            kind: GpuFaultKind::MetalKill,
            raw_message: "code: -3".to_string(),
            timestamp: Instant::now(),
            attempt: 1,
        };
        assert_eq!(fault.kind, GpuFaultKind::MetalKill);
        assert_eq!(fault.attempt, 1);
    }

    #[test]
    fn test_circuit_breaker_record_returns_false_then_true() {
        let mut cb = GpuCircuitBreaker::default();
        assert!(!cb.record_fault());
        assert!(!cb.record_fault());
        assert!(cb.record_fault()); // 3rd fault trips it
        assert!(!cb.record_fault()); // already tripped, returns false
    }
}
