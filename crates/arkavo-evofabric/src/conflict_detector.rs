use crate::target_scope_overlap::overlaps;
use crate::{AstOp, OpBundle, TargetScope};

/// Result of comparing two OpBundles for conflicts.
#[derive(Debug, PartialEq, Eq)]
pub enum ConflictResult {
    /// Operations target completely disjoint scopes — safe to compose both.
    Disjoint,
    /// Operations target the same scope — identical bundles, deduplicate.
    Identical,
    /// Operations target overlapping scopes — needs Critic arbitration.
    Overlapping { scopes: Vec<TargetScope> },
}

/// Detect the conflict relationship between two OpBundles.
pub fn detect_conflict(a: &OpBundle, b: &OpBundle) -> ConflictResult {
    if a.id == b.id {
        return ConflictResult::Identical;
    }

    // Check if content hashes match (same operations, different IDs)
    if a.content_hash != [0u8; 32]
        && b.content_hash != [0u8; 32]
        && a.content_hash == b.content_hash
    {
        return ConflictResult::Identical;
    }

    let overlapping = find_overlapping(a, b);
    if overlapping.is_empty() {
        ConflictResult::Disjoint
    } else {
        ConflictResult::Overlapping {
            scopes: overlapping,
        }
    }
}

/// Compose two disjoint bundles into a combined operation list.
///
/// Returns the merged ops from both bundles. Only valid when bundles are disjoint.
pub fn resolve_disjoint(a: &OpBundle, b: &OpBundle) -> Vec<AstOp> {
    let mut ops = a.ops.clone();
    ops.extend(b.ops.iter().cloned());
    ops
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

fn find_overlapping(a: &OpBundle, b: &OpBundle) -> Vec<TargetScope> {
    let scopes_a = extract_scopes(&a.ops);
    let scopes_b = extract_scopes(&b.ops);
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
    use chrono::Utc;
    use uuid::Uuid;

    fn bundle_with_ops(ops: Vec<AstOp>) -> OpBundle {
        OpBundle {
            id: Uuid::new_v4(),
            originator: "agent".into(),
            target_file: "lib.rs".into(),
            ops,
            rationale: "test".into(),
            created_at: Utc::now(),
            content_hash: [0u8; 32],
            status: OpBundleStatus::Proposed,
        }
    }

    #[test]
    fn disjoint_functions() {
        let a = bundle_with_ops(vec![AstOp::AddAttribute {
            scope: TargetScope::Function {
                name: "foo".into(),
                within: None,
            },
            attribute: "#[inline]".into(),
        }]);
        let b = bundle_with_ops(vec![AstOp::AddAttribute {
            scope: TargetScope::Function {
                name: "bar".into(),
                within: None,
            },
            attribute: "#[inline]".into(),
        }]);
        assert_eq!(detect_conflict(&a, &b), ConflictResult::Disjoint);
    }

    #[test]
    fn same_id_is_identical() {
        let mut a = bundle_with_ops(vec![]);
        let b = a.clone();
        assert_eq!(detect_conflict(&a, &b), ConflictResult::Identical);
    }

    #[test]
    fn same_content_hash_is_identical() {
        let mut a = bundle_with_ops(vec![]);
        a.content_hash = [42u8; 32];
        let mut b = bundle_with_ops(vec![]);
        b.content_hash = [42u8; 32];
        assert_eq!(detect_conflict(&a, &b), ConflictResult::Identical);
    }

    #[test]
    fn overlapping_same_function() {
        let a = bundle_with_ops(vec![AstOp::ReplaceFnBody {
            scope: TargetScope::Function {
                name: "process".into(),
                within: None,
            },
            new_body: "42".into(),
        }]);
        let b = bundle_with_ops(vec![AstOp::ReplaceFnBody {
            scope: TargetScope::Function {
                name: "process".into(),
                within: None,
            },
            new_body: "99".into(),
        }]);
        match detect_conflict(&a, &b) {
            ConflictResult::Overlapping { scopes } => {
                assert_eq!(scopes.len(), 1);
                assert!(
                    matches!(&scopes[0], TargetScope::Function { name, .. } if name == "process")
                );
            }
            other => panic!("Expected Overlapping, got {other:?}"),
        }
    }

    #[test]
    fn resolve_disjoint_combines_ops() {
        let a = bundle_with_ops(vec![AstOp::AddAttribute {
            scope: TargetScope::Function {
                name: "foo".into(),
                within: None,
            },
            attribute: "#[inline]".into(),
        }]);
        let b = bundle_with_ops(vec![AstOp::AddAttribute {
            scope: TargetScope::Function {
                name: "bar".into(),
                within: None,
            },
            attribute: "#[must_use]".into(),
        }]);
        let combined = resolve_disjoint(&a, &b);
        assert_eq!(combined.len(), 2);
    }

    #[test]
    fn add_use_ops_always_disjoint() {
        let a = bundle_with_ops(vec![AstOp::AddUse {
            path: "std::io".into(),
        }]);
        let b = bundle_with_ops(vec![AstOp::AddUse {
            path: "std::fmt".into(),
        }]);
        assert_eq!(detect_conflict(&a, &b), ConflictResult::Disjoint);
    }

    #[test]
    fn impl_block_overlaps_scoped_function() {
        let a = bundle_with_ops(vec![AstOp::Remove {
            scope: TargetScope::ImplBlock {
                type_name: "Config".into(),
                trait_name: None,
            },
        }]);
        let b = bundle_with_ops(vec![AstOp::AddAttribute {
            scope: TargetScope::Function {
                name: "validate".into(),
                within: Some(Box::new(TargetScope::ImplBlock {
                    type_name: "Config".into(),
                    trait_name: None,
                })),
            },
            attribute: "#[inline]".into(),
        }]);
        match detect_conflict(&a, &b) {
            ConflictResult::Overlapping { scopes } => assert!(!scopes.is_empty()),
            other => panic!("Expected Overlapping, got {other:?}"),
        }
    }

    #[test]
    fn file_scope_overlaps_anything() {
        let a = bundle_with_ops(vec![AstOp::InsertAfter {
            scope: TargetScope::File,
            new_item: "fn new_fn() {}".into(),
        }]);
        let b = bundle_with_ops(vec![AstOp::AddAttribute {
            scope: TargetScope::Function {
                name: "foo".into(),
                within: None,
            },
            attribute: "#[inline]".into(),
        }]);
        match detect_conflict(&a, &b) {
            ConflictResult::Overlapping { .. } => {}
            other => panic!("Expected Overlapping, got {other:?}"),
        }
    }

    #[test]
    fn mixed_ops_partial_overlap() {
        let a = bundle_with_ops(vec![
            AstOp::AddAttribute {
                scope: TargetScope::Function {
                    name: "foo".into(),
                    within: None,
                },
                attribute: "#[inline]".into(),
            },
            AstOp::AddUse {
                path: "std::io".into(),
            },
        ]);
        let b = bundle_with_ops(vec![AstOp::ReplaceFnBody {
            scope: TargetScope::Function {
                name: "foo".into(),
                within: None,
            },
            new_body: "42".into(),
        }]);
        match detect_conflict(&a, &b) {
            ConflictResult::Overlapping { scopes } => {
                assert_eq!(scopes.len(), 1);
            }
            other => panic!("Expected Overlapping, got {other:?}"),
        }
    }

    #[test]
    fn zero_content_hash_not_treated_as_identical() {
        let a = bundle_with_ops(vec![]);
        let b = bundle_with_ops(vec![]);
        // Both have [0;32] content hash — should NOT be treated as identical
        // because [0;32] is the "unset" sentinel
        assert_ne!(a.id, b.id);
        assert_eq!(detect_conflict(&a, &b), ConflictResult::Disjoint);
    }

    #[test]
    fn empty_bundles_disjoint() {
        let a = bundle_with_ops(vec![]);
        let b = bundle_with_ops(vec![]);
        assert_eq!(detect_conflict(&a, &b), ConflictResult::Disjoint);
    }
}
