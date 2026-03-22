//! Global GPU inference scheduler.
//!
//! Gates concurrent GPU inference across all agents on a device using an async
//! semaphore. Default capacity: 1 (serialize completely to avoid GPU thrashing).

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::{Semaphore, SemaphorePermit};

/// Global GPU inference scheduler.
///
/// All local model inference funnels through `acquire()`, which parks the
/// calling task until a GPU slot is available. The RAII guard released on
/// drop ensures the slot is freed even on panic.
pub struct GpuScheduler {
    semaphore: Semaphore,
    active_count: AtomicU32,
    total_acquisitions: AtomicU32,
    total_wait_ns: AtomicU64,
    contention_count: AtomicU32,
}

impl GpuScheduler {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            semaphore: Semaphore::new(max_concurrent),
            active_count: AtomicU32::new(0),
            total_acquisitions: AtomicU32::new(0),
            total_wait_ns: AtomicU64::new(0),
            contention_count: AtomicU32::new(0),
        }
    }

    /// Acquire a GPU inference slot. Parks task until available.
    pub async fn acquire(&self, agent_id: &str) -> GpuInferenceGuard<'_> {
        let start = Instant::now();
        let permit = self
            .semaphore
            .acquire()
            .await
            .expect("GPU scheduler semaphore closed");
        let wait_ns = start.elapsed().as_nanos() as u64;

        let active = self.active_count.fetch_add(1, Ordering::Relaxed) + 1;
        self.total_acquisitions.fetch_add(1, Ordering::Relaxed);
        self.total_wait_ns.fetch_add(wait_ns, Ordering::Relaxed);

        // Contention: waited >100ms for a slot
        if wait_ns > 100_000_000 {
            self.contention_count.fetch_add(1, Ordering::Relaxed);
            let wait_ms = wait_ns / 1_000_000;
            crate::subsystem_timing::global_timing()
                .inference
                .record(wait_ms);
            tracing::info!(
                agent_id,
                wait_ms,
                active,
                "GPU scheduler: contention detected"
            );
        }

        GpuInferenceGuard {
            scheduler: self,
            _permit: permit,
        }
    }

    /// Number of inferences currently running on the GPU.
    pub fn active_count(&self) -> u32 {
        self.active_count.load(Ordering::Relaxed)
    }

    /// Lifetime count of waits exceeding the contention threshold.
    pub fn contention_count(&self) -> u32 {
        self.contention_count.load(Ordering::Relaxed)
    }

    /// Total inference acquisitions since startup.
    pub fn total_acquisitions(&self) -> u32 {
        self.total_acquisitions.load(Ordering::Relaxed)
    }

    /// Cumulative wait time in nanoseconds.
    pub fn total_wait_ns(&self) -> u64 {
        self.total_wait_ns.load(Ordering::Relaxed)
    }
}

/// RAII guard — decrements active count and releases semaphore permit on drop.
pub struct GpuInferenceGuard<'a> {
    scheduler: &'a GpuScheduler,
    _permit: SemaphorePermit<'a>,
}

impl Drop for GpuInferenceGuard<'_> {
    fn drop(&mut self) {
        self.scheduler.active_count.fetch_sub(1, Ordering::Relaxed);
    }
}

static GLOBAL_GPU: LazyLock<GpuScheduler> = LazyLock::new(|| GpuScheduler::new(1));

pub fn global_gpu() -> &'static GpuScheduler {
    &GLOBAL_GPU
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_acquire_release() {
        let scheduler = GpuScheduler::new(1);
        assert_eq!(scheduler.active_count(), 0);

        let guard = scheduler.acquire("test-agent").await;
        assert_eq!(scheduler.active_count(), 1);
        assert_eq!(scheduler.total_acquisitions(), 1);

        drop(guard);
        assert_eq!(scheduler.active_count(), 0);
    }

    #[tokio::test]
    async fn test_serialization() {
        let scheduler = GpuScheduler::new(1);
        let guard1 = scheduler.acquire("agent-1").await;
        assert_eq!(scheduler.active_count(), 1);

        // Second acquire would block — verify with try_acquire
        let result = scheduler.semaphore.try_acquire();
        assert!(result.is_err()); // no permit available

        drop(guard1);
        let _guard2 = scheduler.acquire("agent-2").await;
        assert_eq!(scheduler.active_count(), 1);
        assert_eq!(scheduler.total_acquisitions(), 2);
    }
}
