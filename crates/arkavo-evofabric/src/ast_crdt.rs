use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::capability_token::{CapScope, CapabilityToken};
use crate::target_scope_overlap::overlaps;
use crate::vector_clock::VectorClock;
use crate::{AstOp, Error, OpBundle, Result, RustOp, TargetScope, render_file};

/// CRDT for managing concurrent AST modifications from multiple agents.
///
/// Holds the canonical source, a vector clock for causal ordering,
/// and pending/accepted operation queues for conflict-aware merging.
#[derive(Debug)]
pub struct AstCrdt {
    canonical_source: String,
    clock: VectorClock,
    pending: Vec<PendingOp>,
    accepted: Vec<AcceptedOp>,
}

/// An operation waiting for acceptance.
#[derive(Debug, Clone)]
pub struct PendingOp {
    pub bundle: OpBundle,
    pub proposed_at: DateTime<Utc>,
    pub proposer_clock: VectorClock,
}

/// An accepted operation ready to be applied.
#[derive(Debug, Clone)]
pub struct AcceptedOp {
    pub bundle: OpBundle,
    pub accepted_at: DateTime<Utc>,
    pub merge_clock: VectorClock,
}

/// A pair of pending operations that conflict.
#[derive(Debug)]
pub struct ConflictPair {
    pub bundle_a: Uuid,
    pub bundle_b: Uuid,
    pub overlapping_scopes: Vec<TargetScope>,
}

impl AstCrdt {
    /// Create a new CRDT from initial source code.
    pub fn new(source: String) -> Self {
        Self {
            canonical_source: source,
            clock: VectorClock::new(),
            pending: Vec::new(),
            accepted: Vec::new(),
        }
    }

    /// Get the current canonical source.
    pub fn canonical_source(&self) -> &str {
        &self.canonical_source
    }

    /// Get the current vector clock.
    pub fn clock(&self) -> &VectorClock {
        &self.clock
    }

    /// Get pending operations.
    pub fn pending(&self) -> &[PendingOp] {
        &self.pending
    }

    /// Get accepted operations.
    pub fn accepted(&self) -> &[AcceptedOp] {
        &self.accepted
    }

    /// Propose an OpBundle for inclusion.
    ///
    /// Validates the capability token has Edit scope and is not expired.
    pub fn propose(
        &mut self,
        bundle: OpBundle,
        agent_id: &str,
        token: &CapabilityToken,
    ) -> Result<()> {
        if !token.has_scope(CapScope::Edit) {
            return Err(Error::CapabilityDenied(
                "token lacks Edit capability".into(),
            ));
        }
        if token.is_expired() {
            return Err(Error::CapabilityDenied("token has expired".into()));
        }

        self.clock.increment(agent_id);
        let proposer_clock = self.clock.clone();

        self.pending.push(PendingOp {
            bundle,
            proposed_at: Utc::now(),
            proposer_clock,
        });

        Ok(())
    }

    /// Accept a pending bundle by ID, moving it to the accepted queue.
    pub fn accept(&mut self, bundle_id: Uuid, agent_id: &str) -> Result<()> {
        let idx = self
            .pending
            .iter()
            .position(|p| p.bundle.id == bundle_id)
            .ok_or_else(|| Error::TargetNotFound(format!("pending bundle {bundle_id}")))?;

        let pending = self.pending.remove(idx);
        self.clock.increment(agent_id);
        let merge_clock = self.clock.clone();

        self.accepted.push(AcceptedOp {
            bundle: pending.bundle,
            accepted_at: Utc::now(),
            merge_clock,
        });

        Ok(())
    }

    /// Detect conflicts among pending operations.
    ///
    /// Two bundles conflict if any of their operations target overlapping scopes.
    pub fn pending_conflicts(&self) -> Vec<ConflictPair> {
        let mut conflicts = Vec::new();
        for i in 0..self.pending.len() {
            for j in (i + 1)..self.pending.len() {
                let scopes = find_overlapping_scopes(
                    &self.pending[i].bundle.ops,
                    &self.pending[j].bundle.ops,
                );
                if !scopes.is_empty() {
                    conflicts.push(ConflictPair {
                        bundle_a: self.pending[i].bundle.id,
                        bundle_b: self.pending[j].bundle.id,
                        overlapping_scopes: scopes,
                    });
                }
            }
        }
        conflicts
    }

    /// Apply all accepted operations in order to the canonical source.
    ///
    /// Returns the rendered source after applying all accepted bundles.
    /// Clears the accepted queue on success.
    pub fn apply_accepted(&mut self) -> Result<String> {
        let mut source = self.canonical_source.clone();
        for accepted in self.accepted.drain(..) {
            let file = RustOp::apply_bundle(&source, &accepted.bundle)?;
            source = render_file(&file);
        }
        self.canonical_source.clone_from(&source);
        Ok(source)
    }

    /// Reject a pending bundle by ID.
    pub fn reject(&mut self, bundle_id: Uuid) -> Result<()> {
        let idx = self
            .pending
            .iter()
            .position(|p| p.bundle.id == bundle_id)
            .ok_or_else(|| Error::TargetNotFound(format!("pending bundle {bundle_id}")))?;
        self.pending.remove(idx);
        Ok(())
    }
}

fn extract_scopes(ops: &[AstOp]) -> Vec<&TargetScope> {
    ops.iter()
        .filter_map(|op| match op {
            AstOp::ReplaceFnBody { scope, .. }
            | AstOp::InsertAfter { scope, .. }
            | AstOp::Remove { scope }
            | AstOp::ReplaceItem { scope, .. }
            | AstOp::AddAttribute { scope, .. } => Some(scope),
            AstOp::AddUse { .. } => None,
        })
        .collect()
}

fn find_overlapping_scopes(ops_a: &[AstOp], ops_b: &[AstOp]) -> Vec<TargetScope> {
    let scopes_a = extract_scopes(ops_a);
    let scopes_b = extract_scopes(ops_b);
    let mut result = Vec::new();
    for sa in &scopes_a {
        for sb in &scopes_b {
            if overlaps(sa, sb) && !result.iter().any(|r: &TargetScope| r == *sa) {
                result.push((*sa).clone());
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpBundleStatus;
    use crate::capability_token::CapabilityToken;

    fn edit_token() -> CapabilityToken {
        CapabilityToken::new(
            "agent-1".into(),
            [CapScope::Edit].into_iter().collect(),
            "root".into(),
            Utc::now() + chrono::Duration::hours(1),
        )
    }

    fn expired_token() -> CapabilityToken {
        CapabilityToken::new(
            "agent-1".into(),
            [CapScope::Edit].into_iter().collect(),
            "root".into(),
            Utc::now() - chrono::Duration::hours(1),
        )
    }

    fn test_only_token() -> CapabilityToken {
        CapabilityToken::new(
            "agent-1".into(),
            [CapScope::Test].into_iter().collect(),
            "root".into(),
            Utc::now() + chrono::Duration::hours(1),
        )
    }

    fn make_bundle(ops: Vec<AstOp>) -> OpBundle {
        OpBundle {
            id: Uuid::new_v4(),
            originator: "agent-1".into(),
            target_file: "lib.rs".into(),
            ops,
            rationale: "test".into(),
            created_at: Utc::now(),
            content_hash: [0u8; 32],
            status: OpBundleStatus::Proposed,
        }
    }

    #[test]
    fn propose_with_edit_token() {
        let mut crdt = AstCrdt::new("fn foo() {}".into());
        let bundle = make_bundle(vec![AstOp::AddAttribute {
            scope: TargetScope::Function {
                name: "foo".into(),
                within: None,
            },
            attribute: "#[inline]".into(),
        }]);
        let token = edit_token();
        assert!(crdt.propose(bundle, "agent-1", &token).is_ok());
        assert_eq!(crdt.pending().len(), 1);
    }

    #[test]
    fn propose_denied_without_edit() {
        let mut crdt = AstCrdt::new("fn foo() {}".into());
        let bundle = make_bundle(vec![]);
        let token = test_only_token();
        let err = crdt.propose(bundle, "agent-1", &token).unwrap_err();
        assert!(err.to_string().contains("Edit"));
    }

    #[test]
    fn propose_denied_expired() {
        let mut crdt = AstCrdt::new("fn foo() {}".into());
        let bundle = make_bundle(vec![]);
        let token = expired_token();
        let err = crdt.propose(bundle, "agent-1", &token).unwrap_err();
        assert!(err.to_string().contains("expired"));
    }

    #[test]
    fn accept_moves_to_accepted() {
        let mut crdt = AstCrdt::new("fn foo() {}".into());
        let bundle = make_bundle(vec![]);
        let id = bundle.id;
        let token = edit_token();
        crdt.propose(bundle, "agent-1", &token).unwrap();
        crdt.accept(id, "merger-1").unwrap();
        assert_eq!(crdt.pending().len(), 0);
        assert_eq!(crdt.accepted().len(), 1);
    }

    #[test]
    fn accept_missing_bundle_errors() {
        let mut crdt = AstCrdt::new("fn foo() {}".into());
        let err = crdt.accept(Uuid::new_v4(), "merger-1").unwrap_err();
        assert!(err.to_string().contains("pending bundle"));
    }

    #[test]
    fn detect_no_conflicts() {
        let mut crdt = AstCrdt::new("fn foo() {} fn bar() {}".into());
        let token = edit_token();
        let b1 = make_bundle(vec![AstOp::AddAttribute {
            scope: TargetScope::Function {
                name: "foo".into(),
                within: None,
            },
            attribute: "#[inline]".into(),
        }]);
        let b2 = make_bundle(vec![AstOp::AddAttribute {
            scope: TargetScope::Function {
                name: "bar".into(),
                within: None,
            },
            attribute: "#[inline]".into(),
        }]);
        crdt.propose(b1, "a1", &token).unwrap();
        crdt.propose(b2, "a2", &token).unwrap();
        assert!(crdt.pending_conflicts().is_empty());
    }

    #[test]
    fn detect_conflict_same_scope() {
        let mut crdt = AstCrdt::new("fn foo() {}".into());
        let token = edit_token();
        let b1 = make_bundle(vec![AstOp::AddAttribute {
            scope: TargetScope::Function {
                name: "foo".into(),
                within: None,
            },
            attribute: "#[inline]".into(),
        }]);
        let b2 = make_bundle(vec![AstOp::AddAttribute {
            scope: TargetScope::Function {
                name: "foo".into(),
                within: None,
            },
            attribute: "#[must_use]".into(),
        }]);
        crdt.propose(b1, "a1", &token).unwrap();
        crdt.propose(b2, "a2", &token).unwrap();
        let conflicts = crdt.pending_conflicts();
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn apply_accepted_updates_source() {
        let source = "fn foo() { let x = 1; }\n";
        let mut crdt = AstCrdt::new(source.into());
        let token = edit_token();
        let bundle = make_bundle(vec![AstOp::AddAttribute {
            scope: TargetScope::Function {
                name: "foo".into(),
                within: None,
            },
            attribute: "#[inline]".into(),
        }]);
        let id = bundle.id;
        crdt.propose(bundle, "a1", &token).unwrap();
        crdt.accept(id, "m1").unwrap();
        let result = crdt.apply_accepted().unwrap();
        assert!(result.contains("#[inline]"));
        assert!(crdt.canonical_source().contains("#[inline]"));
    }

    #[test]
    fn clock_increments_on_propose_and_accept() {
        let mut crdt = AstCrdt::new("fn foo() {}".into());
        let token = edit_token();
        let bundle = make_bundle(vec![]);
        let id = bundle.id;
        crdt.propose(bundle, "agent-1", &token).unwrap();
        assert_eq!(crdt.clock().get("agent-1"), 1);
        crdt.accept(id, "merger-1").unwrap();
        assert_eq!(crdt.clock().get("merger-1"), 1);
    }

    #[test]
    fn reject_removes_from_pending() {
        let mut crdt = AstCrdt::new("fn foo() {}".into());
        let token = edit_token();
        let bundle = make_bundle(vec![]);
        let id = bundle.id;
        crdt.propose(bundle, "a1", &token).unwrap();
        crdt.reject(id).unwrap();
        assert!(crdt.pending().is_empty());
    }

    #[test]
    fn reject_missing_errors() {
        let mut crdt = AstCrdt::new("fn foo() {}".into());
        assert!(crdt.reject(Uuid::new_v4()).is_err());
    }

    #[test]
    fn add_use_ops_dont_conflict() {
        let mut crdt = AstCrdt::new("fn foo() {}".into());
        let token = edit_token();
        let b1 = make_bundle(vec![AstOp::AddUse {
            path: "std::collections::HashMap".into(),
        }]);
        let b2 = make_bundle(vec![AstOp::AddUse {
            path: "std::io::Read".into(),
        }]);
        crdt.propose(b1, "a1", &token).unwrap();
        crdt.propose(b2, "a2", &token).unwrap();
        assert!(crdt.pending_conflicts().is_empty());
    }

    #[test]
    fn multiple_agents_concurrent_clocks() {
        let mut crdt = AstCrdt::new("fn foo() {}".into());
        let token = edit_token();
        let b1 = make_bundle(vec![]);
        let b2 = make_bundle(vec![]);
        crdt.propose(b1, "agent-a", &token).unwrap();
        crdt.propose(b2, "agent-b", &token).unwrap();
        assert_eq!(crdt.clock().get("agent-a"), 1);
        assert_eq!(crdt.clock().get("agent-b"), 1);
    }
}
