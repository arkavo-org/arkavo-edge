//! Adaptive layer for fast runtime updates
//!
//! The adaptive layer supports rapid threshold and weight adjustments
//! with automatic rollback capability.

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::{SbeError, SbeResult};
use crate::patchlet::{AppliedPatchlet, DEFAULT_MAX_PATCHLETS, Patchlet};

/// Default rollback timeout in milliseconds
pub const DEFAULT_ROLLBACK_TIMEOUT_MS: u64 = 100;

/// Handle for rolling back a patchlet
#[derive(Debug, Clone)]
pub struct RollbackHandle {
    /// ID of the patchlet that can be rolled back
    pub patchlet_id: Uuid,
    /// When the rollback expires
    expiry: Instant,
}

impl RollbackHandle {
    /// Create a new rollback handle
    fn new(patchlet_id: Uuid, timeout_ms: u64) -> Self {
        Self {
            patchlet_id,
            expiry: Instant::now() + Duration::from_millis(timeout_ms),
        }
    }

    /// Check if this handle has expired
    #[must_use]
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expiry
    }

    /// Get remaining time before expiry
    #[must_use]
    pub fn remaining(&self) -> Duration {
        self.expiry.saturating_duration_since(Instant::now())
    }
}

/// The adaptive layer for fast runtime updates
#[derive(Debug, Clone)]
pub struct AdaptiveLayer {
    /// Applied patchlets in order
    patchlets: Vec<AppliedPatchlet>,
    /// Rollback timeout in milliseconds
    rollback_timeout_ms: u64,
    /// Maximum number of patchlets before recompile
    max_patchlets: usize,
    /// When the layer was created
    pub created_at: DateTime<Utc>,
}

impl Default for AdaptiveLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl AdaptiveLayer {
    /// Create a new adaptive layer with default settings
    #[must_use]
    pub fn new() -> Self {
        Self {
            patchlets: Vec::new(),
            rollback_timeout_ms: DEFAULT_ROLLBACK_TIMEOUT_MS,
            max_patchlets: DEFAULT_MAX_PATCHLETS,
            created_at: Utc::now(),
        }
    }

    /// Create an adaptive layer with custom settings
    #[must_use]
    pub fn with_config(rollback_timeout_ms: u64, max_patchlets: usize) -> Self {
        Self {
            patchlets: Vec::new(),
            rollback_timeout_ms,
            max_patchlets,
            created_at: Utc::now(),
        }
    }

    /// Get the rollback timeout in milliseconds
    #[must_use]
    pub const fn rollback_timeout_ms(&self) -> u64 {
        self.rollback_timeout_ms
    }

    /// Get the maximum number of patchlets
    #[must_use]
    pub const fn max_patchlets(&self) -> usize {
        self.max_patchlets
    }

    /// Get all applied patchlets
    #[must_use]
    pub fn patchlets(&self) -> &[AppliedPatchlet] {
        &self.patchlets
    }

    /// Get the number of applied patchlets
    #[must_use]
    pub fn patchlet_count(&self) -> usize {
        self.patchlets.len()
    }

    /// Check if a recompile is required
    #[must_use]
    pub fn should_trigger_recompile(&self) -> bool {
        self.patchlets.len() >= self.max_patchlets
    }

    /// Apply a patchlet to this layer
    pub fn apply(&mut self, patchlet: Patchlet) -> SbeResult<RollbackHandle> {
        if self.should_trigger_recompile() {
            return Err(SbeError::RecompileRequired {
                count: self.patchlets.len(),
                max: self.max_patchlets,
            });
        }

        let patchlet_id = patchlet.id;
        let applied = AppliedPatchlet::new(patchlet);
        self.patchlets.push(applied);

        Ok(RollbackHandle::new(patchlet_id, self.rollback_timeout_ms))
    }

    /// Rollback a patchlet using its handle
    pub fn rollback(&mut self, handle: &RollbackHandle) -> SbeResult<Patchlet> {
        if handle.is_expired() {
            return Err(SbeError::RollbackExpired(handle.patchlet_id));
        }

        let position = self
            .patchlets
            .iter()
            .position(|p| p.patchlet.id == handle.patchlet_id && p.rollback_available);

        match position {
            Some(idx) => {
                let applied = self.patchlets.remove(idx);
                Ok(applied.patchlet)
            }
            None => Err(SbeError::RollbackExpired(handle.patchlet_id)),
        }
    }

    /// Expire rollback for a specific patchlet
    pub fn expire_rollback(&mut self, patchlet_id: &Uuid) {
        for applied in &mut self.patchlets {
            if applied.patchlet.id == *patchlet_id {
                applied.expire_rollback();
                break;
            }
        }
    }

    /// Expire all rollbacks
    pub fn expire_all_rollbacks(&mut self) {
        for applied in &mut self.patchlets {
            applied.expire_rollback();
        }
    }

    /// Clear all patchlets (for recompilation)
    pub fn clear(&mut self) {
        self.patchlets.clear();
    }

    /// Get a patchlet by ID
    #[must_use]
    pub fn get_patchlet(&self, id: &Uuid) -> Option<&AppliedPatchlet> {
        self.patchlets.iter().find(|p| p.patchlet.id == *id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patchlet::PatchOp;
    use std::thread;

    #[test]
    fn test_adaptive_layer_creation() {
        let layer = AdaptiveLayer::new();
        assert_eq!(layer.rollback_timeout_ms(), DEFAULT_ROLLBACK_TIMEOUT_MS);
        assert_eq!(layer.max_patchlets(), DEFAULT_MAX_PATCHLETS);
        assert_eq!(layer.patchlet_count(), 0);
        assert!(!layer.should_trigger_recompile());
    }

    #[test]
    fn test_custom_config() {
        let layer = AdaptiveLayer::with_config(200, 50);
        assert_eq!(layer.rollback_timeout_ms(), 200);
        assert_eq!(layer.max_patchlets(), 50);
    }

    #[test]
    fn test_apply_patchlet() {
        let mut layer = AdaptiveLayer::new();
        let patchlet = Patchlet::new(
            vec![1],
            PatchOp::AdjustThreshold {
                node_id: 1,
                new_threshold: 0.5,
            },
        )
        .unwrap();

        let handle = layer.apply(patchlet).unwrap();
        assert_eq!(layer.patchlet_count(), 1);
        assert!(!handle.is_expired());
    }

    #[test]
    fn test_rollback() {
        let mut layer = AdaptiveLayer::with_config(5000, 20);
        let patchlet = Patchlet::new(
            vec![1],
            PatchOp::AdjustThreshold {
                node_id: 1,
                new_threshold: 0.5,
            },
        )
        .unwrap();

        let handle = layer.apply(patchlet).unwrap();
        assert_eq!(layer.patchlet_count(), 1);

        let rolled_back = layer.rollback(&handle).unwrap();
        assert_eq!(layer.patchlet_count(), 0);
        assert_eq!(rolled_back.target_nodes, vec![1]);
    }

    #[test]
    fn test_rollback_expired() {
        let mut layer = AdaptiveLayer::with_config(1, 20);
        let patchlet = Patchlet::new(
            vec![1],
            PatchOp::AdjustThreshold {
                node_id: 1,
                new_threshold: 0.5,
            },
        )
        .unwrap();

        let handle = layer.apply(patchlet).unwrap();

        thread::sleep(Duration::from_millis(10));

        assert!(handle.is_expired());
        assert!(layer.rollback(&handle).is_err());
    }

    #[test]
    fn test_recompile_required() {
        let mut layer = AdaptiveLayer::with_config(100, 3);

        for i in 0..3 {
            let patchlet = Patchlet::new(
                vec![i],
                PatchOp::AdjustThreshold {
                    node_id: i,
                    new_threshold: 0.5,
                },
            )
            .unwrap();
            layer.apply(patchlet).unwrap();
        }

        assert!(layer.should_trigger_recompile());

        let patchlet = Patchlet::new(
            vec![99],
            PatchOp::AdjustThreshold {
                node_id: 99,
                new_threshold: 0.5,
            },
        )
        .unwrap();

        assert!(matches!(
            layer.apply(patchlet),
            Err(SbeError::RecompileRequired { .. })
        ));
    }

    #[test]
    fn test_expire_rollback() {
        let mut layer = AdaptiveLayer::with_config(10000, 20);
        let patchlet = Patchlet::new(
            vec![1],
            PatchOp::AdjustThreshold {
                node_id: 1,
                new_threshold: 0.5,
            },
        )
        .unwrap();
        let patchlet_id = patchlet.id;

        layer.apply(patchlet).unwrap();
        layer.expire_rollback(&patchlet_id);

        let applied = layer.get_patchlet(&patchlet_id).unwrap();
        assert!(!applied.rollback_available);
    }

    #[test]
    fn test_clear() {
        let mut layer = AdaptiveLayer::new();
        for i in 0..5 {
            let patchlet = Patchlet::new(
                vec![i],
                PatchOp::AdjustThreshold {
                    node_id: i,
                    new_threshold: 0.5,
                },
            )
            .unwrap();
            layer.apply(patchlet).unwrap();
        }

        assert_eq!(layer.patchlet_count(), 5);
        layer.clear();
        assert_eq!(layer.patchlet_count(), 0);
    }
}
