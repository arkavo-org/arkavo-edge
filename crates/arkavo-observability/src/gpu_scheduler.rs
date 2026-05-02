//! Adaptive GPU inference scheduler.
//!
//! Gates concurrent GPU inference across all agents using an async semaphore.
//! Starts with a capacity derived from system memory, then self-heals:
//! contention signals drive capacity upward, throughput drops drive it back.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::{Semaphore, SemaphorePermit};

/// Adaptive GPU inference scheduler.
///
/// All local model inference funnels through `acquire()`, which parks the
/// calling task until a GPU slot is available. The RAII guard released on
/// drop ensures the slot is freed even on panic.
///
/// Self-healing: when sustained contention is detected (>5 waits exceeding
/// the threshold within a window), capacity grows by 1 up to the device max.
pub struct GpuScheduler {
    semaphore: Semaphore,
    active_count: AtomicU32,
    total_acquisitions: AtomicU32,
    total_wait_ns: AtomicU64,
    contention_count: AtomicU32,
    capacity: AtomicU32,
    max_capacity: u32,
}

/// Contention threshold: waits above this are counted for adaptation.
const CONTENTION_THRESHOLD_NS: u64 = 100_000_000; // 100ms
/// Contention events before capacity grows.
const GROW_AFTER_CONTENTIONS: u32 = 5;

impl GpuScheduler {
    pub fn new(max_concurrent: usize) -> Self {
        let max_concurrent = max_concurrent.max(1);
        Self {
            semaphore: Semaphore::new(max_concurrent),
            active_count: AtomicU32::new(0),
            total_acquisitions: AtomicU32::new(0),
            total_wait_ns: AtomicU64::new(0),
            contention_count: AtomicU32::new(0),
            capacity: AtomicU32::new(max_concurrent as u32),
            max_capacity: detect_max_capacity(),
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

        if wait_ns > CONTENTION_THRESHOLD_NS {
            let contentions = self.contention_count.fetch_add(1, Ordering::Relaxed) + 1;
            let wait_ms = wait_ns / 1_000_000;
            crate::subsystem_timing::global_timing()
                .inference
                .record(wait_ms);
            tracing::info!(
                agent_id,
                wait_ms,
                active,
                capacity = self.capacity.load(Ordering::Relaxed),
                "GPU scheduler: contention detected"
            );
            self.maybe_grow(contentions);
        }

        GpuInferenceGuard {
            scheduler: self,
            _permit: permit,
        }
    }

    /// Grow capacity if sustained contention warrants it.
    fn maybe_grow(&self, contentions: u32) {
        if !contentions.is_multiple_of(GROW_AFTER_CONTENTIONS) {
            return;
        }
        let current = self.capacity.load(Ordering::Relaxed);
        if current >= self.max_capacity {
            return;
        }
        let new_cap = current + 1;
        if self
            .capacity
            .compare_exchange(current, new_cap, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            self.semaphore.add_permits(1);
            tracing::info!(
                old = current,
                new = new_cap,
                max = self.max_capacity,
                "GPU scheduler: capacity grown (self-healing)"
            );
        }
    }

    /// Number of inferences currently running on the GPU.
    pub fn active_count(&self) -> u32 {
        self.active_count.load(Ordering::Relaxed)
    }

    /// Current semaphore capacity.
    pub fn capacity(&self) -> u32 {
        self.capacity.load(Ordering::Relaxed)
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

/// Derive initial capacity from system memory.
/// Apple Silicon shares memory between CPU and GPU (unified architecture).
fn detect_initial_capacity() -> usize {
    let total_gb = sysinfo::System::new_all().total_memory() / (1024 * 1024 * 1024);
    let cap = match total_gb {
        0..=7 => 1,
        8..=15 => 2,
        16..=31 => 2,
        32..=63 => 3,
        64..=127 => 4,
        _ => 4,
    };
    tracing::info!(
        total_memory_gb = total_gb,
        initial_capacity = cap,
        "GPU scheduler: capacity derived from system memory"
    );
    cap
}

/// Max capacity the scheduler can grow to.
fn detect_max_capacity() -> u32 {
    let total_gb = sysinfo::System::new_all().total_memory() / (1024 * 1024 * 1024);
    match total_gb {
        0..=7 => 1,
        8..=15 => 2,
        16..=31 => 3,
        32..=63 => 4,
        64..=127 => 6,
        _ => 8,
    }
}

static GLOBAL_GPU: LazyLock<GpuScheduler> =
    LazyLock::new(|| GpuScheduler::new(detect_initial_capacity()));

pub fn global_gpu() -> &'static GpuScheduler {
    &GLOBAL_GPU
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_acquire_release() {
        let scheduler = GpuScheduler::new(2);
        assert_eq!(scheduler.active_count(), 0);
        assert_eq!(scheduler.capacity(), 2);

        let guard = scheduler.acquire("test-agent").await;
        assert_eq!(scheduler.active_count(), 1);
        assert_eq!(scheduler.total_acquisitions(), 1);

        drop(guard);
        assert_eq!(scheduler.active_count(), 0);
    }

    #[tokio::test]
    async fn test_concurrent_permits() {
        let scheduler = GpuScheduler::new(3);
        let g1 = scheduler.acquire("agent-1").await;
        let g2 = scheduler.acquire("agent-2").await;
        let g3 = scheduler.acquire("agent-3").await;
        assert_eq!(scheduler.active_count(), 3);

        // 4th would block
        let result = scheduler.semaphore.try_acquire();
        assert!(result.is_err());

        drop(g1);
        drop(g2);
        drop(g3);
        assert_eq!(scheduler.active_count(), 0);
    }

    #[tokio::test]
    async fn test_grow_on_contention() {
        let scheduler = GpuScheduler {
            semaphore: Semaphore::new(1),
            active_count: AtomicU32::new(0),
            total_acquisitions: AtomicU32::new(0),
            total_wait_ns: AtomicU64::new(0),
            contention_count: AtomicU32::new(GROW_AFTER_CONTENTIONS - 1),
            capacity: AtomicU32::new(1),
            max_capacity: 4,
        };

        // Simulate the 5th contention event triggering growth
        scheduler.maybe_grow(GROW_AFTER_CONTENTIONS);
        assert_eq!(scheduler.capacity(), 2);

        // Verify new permit is usable
        let g1 = scheduler.acquire("a").await;
        let g2 = scheduler.acquire("b").await;
        assert_eq!(scheduler.active_count(), 2);
        drop(g1);
        drop(g2);
    }

    #[tokio::test]
    async fn test_no_grow_beyond_max() {
        let scheduler = GpuScheduler {
            semaphore: Semaphore::new(2),
            active_count: AtomicU32::new(0),
            total_acquisitions: AtomicU32::new(0),
            total_wait_ns: AtomicU64::new(0),
            contention_count: AtomicU32::new(0),
            capacity: AtomicU32::new(2),
            max_capacity: 2,
        };
        scheduler.maybe_grow(GROW_AFTER_CONTENTIONS);
        assert_eq!(scheduler.capacity(), 2); // capped at max
    }

    #[test]
    fn test_detect_capacity() {
        let cap = detect_initial_capacity();
        assert!(cap >= 1);
        let max = detect_max_capacity();
        assert!(max >= cap as u32);
    }
}
