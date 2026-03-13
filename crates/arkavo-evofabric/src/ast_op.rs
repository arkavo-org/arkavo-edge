use crate::target_scope::TargetScope;
use serde::{Deserialize, Serialize};

/// A typed AST operation that an agent can propose.
///
/// Each operation targets a semantic scope in the source file and carries
/// the transformation as source text (parsed and validated before application).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum AstOp {
    /// Replace a function's body with new source code
    ReplaceFnBody {
        scope: TargetScope,
        new_body: String,
    },

    /// Insert a new item (fn, struct, impl) after the target
    InsertAfter {
        scope: TargetScope,
        new_item: String,
    },

    /// Remove an item matching the scope
    Remove { scope: TargetScope },

    /// Replace an entire item with new source
    ReplaceItem {
        scope: TargetScope,
        new_item: String,
    },

    /// Add an attribute to an item (e.g. `#[inline]`)
    AddAttribute {
        scope: TargetScope,
        attribute: String,
    },

    /// Add a use statement to the file
    AddUse { path: String },
}

impl AstOp {
    /// Human-readable description for evidence logging.
    pub fn describe(&self) -> String {
        match self {
            Self::ReplaceFnBody { scope, .. } => format!("replace body of {scope}"),
            Self::InsertAfter { scope, .. } => format!("insert after {scope}"),
            Self::Remove { scope } => format!("remove {scope}"),
            Self::ReplaceItem { scope, .. } => format!("replace {scope}"),
            Self::AddAttribute { scope, attribute } => {
                format!("add {attribute} to {scope}")
            }
            Self::AddUse { path } => format!("add use {path}"),
        }
    }

    /// Get the target scope of this operation, if any.
    pub fn scope(&self) -> Option<&TargetScope> {
        match self {
            Self::ReplaceFnBody { scope, .. }
            | Self::InsertAfter { scope, .. }
            | Self::Remove { scope }
            | Self::ReplaceItem { scope, .. }
            | Self::AddAttribute { scope, .. } => Some(scope),
            Self::AddUse { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_formatting() {
        let op = AstOp::ReplaceFnBody {
            scope: TargetScope::Function {
                name: "foo".into(),
                within: None,
            },
            new_body: "{ 42 }".into(),
        };
        assert_eq!(op.describe(), "replace body of fn foo");

        let op = AstOp::AddAttribute {
            scope: TargetScope::Function {
                name: "bar".into(),
                within: None,
            },
            attribute: "#[inline]".into(),
        };
        assert_eq!(op.describe(), "add #[inline] to fn bar");

        let op = AstOp::AddUse {
            path: "std::collections::HashMap".into(),
        };
        assert_eq!(op.describe(), "add use std::collections::HashMap");
    }

    #[test]
    fn scope_accessor() {
        let op = AstOp::Remove {
            scope: TargetScope::TypeDef { name: "Foo".into() },
        };
        assert!(op.scope().is_some());

        let op = AstOp::AddUse { path: "foo".into() };
        assert!(op.scope().is_none());
    }

    #[test]
    fn serde_roundtrip() {
        let op = AstOp::ReplaceFnBody {
            scope: TargetScope::Function {
                name: "test".into(),
                within: Some(Box::new(TargetScope::ImplBlock {
                    type_name: "Foo".into(),
                    trait_name: None,
                })),
            },
            new_body: "{ self.x + 1 }".into(),
        };
        let json = serde_json::to_string(&op).unwrap();
        let back: AstOp = serde_json::from_str(&json).unwrap();
        assert_eq!(op, back);
    }
}
