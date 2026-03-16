use crate::TargetScope;

/// Check if two target scopes overlap (could conflict if modified concurrently).
///
/// Two scopes overlap when modifications to both could produce conflicting AST changes.
/// File scope overlaps everything. Same-kind same-name scopes overlap.
/// An ImplBlock overlaps a Function scoped within that impl block.
pub fn overlaps(a: &TargetScope, b: &TargetScope) -> bool {
    if a == b {
        return true;
    }
    match (a, b) {
        // File overlaps everything
        (TargetScope::File, _) | (_, TargetScope::File) => true,

        // Two functions overlap if same name and compatible parents
        (
            TargetScope::Function {
                name: na,
                within: wa,
            },
            TargetScope::Function {
                name: nb,
                within: wb,
            },
        ) => na == nb && within_compatible(wa.as_deref(), wb.as_deref()),

        // Two impl blocks overlap if same type (trait refinement doesn't separate)
        (
            TargetScope::ImplBlock {
                type_name: ta,
                trait_name: _,
            },
            TargetScope::ImplBlock {
                type_name: tb,
                trait_name: _,
            },
        ) => ta == tb,

        // ImplBlock overlaps a function scoped within that impl
        (
            TargetScope::ImplBlock {
                type_name,
                trait_name,
            },
            TargetScope::Function {
                within: Some(parent),
                ..
            },
        )
        | (
            TargetScope::Function {
                within: Some(parent),
                ..
            },
            TargetScope::ImplBlock {
                type_name,
                trait_name,
            },
        ) => impl_scope_matches(parent, type_name, trait_name.as_deref()),

        // Two type defs overlap if same name
        (TargetScope::TypeDef { name: na }, TargetScope::TypeDef { name: nb }) => na == nb,

        // Two use statements overlap if same path
        (TargetScope::UseStatement { path: pa }, TargetScope::UseStatement { path: pb }) => {
            pa == pb
        }

        // All other cross-kind combinations are disjoint
        _ => false,
    }
}

fn within_compatible(wa: Option<&TargetScope>, wb: Option<&TargetScope>) -> bool {
    match (wa, wb) {
        // Both unscoped — same top-level function
        (None, None) => true,
        // One scoped, one not — potentially the same function, conservatively overlap
        (None, Some(_)) | (Some(_), None) => true,
        // Both scoped — check if parent scopes overlap
        (Some(pa), Some(pb)) => overlaps(pa, pb),
    }
}

fn impl_scope_matches(parent: &TargetScope, type_name: &str, trait_name: Option<&str>) -> bool {
    match parent {
        TargetScope::ImplBlock {
            type_name: pt,
            trait_name: ptn,
        } => {
            pt == type_name
                && match (ptn.as_deref(), trait_name) {
                    (None, _) | (_, None) => true,
                    (Some(a), Some(b)) => a == b,
                }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_scopes_overlap() {
        let s = TargetScope::Function {
            name: "foo".into(),
            within: None,
        };
        assert!(overlaps(&s, &s));
    }

    #[test]
    fn file_overlaps_everything() {
        let file = TargetScope::File;
        let func = TargetScope::Function {
            name: "foo".into(),
            within: None,
        };
        let ty = TargetScope::TypeDef { name: "Bar".into() };
        assert!(overlaps(&file, &func));
        assert!(overlaps(&func, &file));
        assert!(overlaps(&file, &ty));
    }

    #[test]
    fn same_function_overlaps() {
        let a = TargetScope::Function {
            name: "process".into(),
            within: None,
        };
        let b = TargetScope::Function {
            name: "process".into(),
            within: None,
        };
        assert!(overlaps(&a, &b));
    }

    #[test]
    fn different_functions_disjoint() {
        let a = TargetScope::Function {
            name: "foo".into(),
            within: None,
        };
        let b = TargetScope::Function {
            name: "bar".into(),
            within: None,
        };
        assert!(!overlaps(&a, &b));
    }

    #[test]
    fn same_impl_block_overlaps() {
        let a = TargetScope::ImplBlock {
            type_name: "Config".into(),
            trait_name: None,
        };
        let b = TargetScope::ImplBlock {
            type_name: "Config".into(),
            trait_name: Some("Display".into()),
        };
        assert!(overlaps(&a, &b));
    }

    #[test]
    fn different_impl_blocks_disjoint() {
        let a = TargetScope::ImplBlock {
            type_name: "Foo".into(),
            trait_name: None,
        };
        let b = TargetScope::ImplBlock {
            type_name: "Bar".into(),
            trait_name: None,
        };
        assert!(!overlaps(&a, &b));
    }

    #[test]
    fn impl_overlaps_scoped_function() {
        let imp = TargetScope::ImplBlock {
            type_name: "Config".into(),
            trait_name: None,
        };
        let func = TargetScope::Function {
            name: "validate".into(),
            within: Some(Box::new(TargetScope::ImplBlock {
                type_name: "Config".into(),
                trait_name: None,
            })),
        };
        assert!(overlaps(&imp, &func));
        assert!(overlaps(&func, &imp));
    }

    #[test]
    fn impl_disjoint_from_different_scoped_function() {
        let imp = TargetScope::ImplBlock {
            type_name: "Config".into(),
            trait_name: None,
        };
        let func = TargetScope::Function {
            name: "validate".into(),
            within: Some(Box::new(TargetScope::ImplBlock {
                type_name: "Other".into(),
                trait_name: None,
            })),
        };
        assert!(!overlaps(&imp, &func));
    }

    #[test]
    fn same_typedef_overlaps() {
        let a = TargetScope::TypeDef {
            name: "Config".into(),
        };
        let b = TargetScope::TypeDef {
            name: "Config".into(),
        };
        assert!(overlaps(&a, &b));
    }

    #[test]
    fn different_typedef_disjoint() {
        let a = TargetScope::TypeDef { name: "Foo".into() };
        let b = TargetScope::TypeDef { name: "Bar".into() };
        assert!(!overlaps(&a, &b));
    }

    #[test]
    fn same_use_overlaps() {
        let a = TargetScope::UseStatement {
            path: "std::collections::HashMap".into(),
        };
        let b = TargetScope::UseStatement {
            path: "std::collections::HashMap".into(),
        };
        assert!(overlaps(&a, &b));
    }

    #[test]
    fn cross_kind_disjoint() {
        let func = TargetScope::Function {
            name: "foo".into(),
            within: None,
        };
        let ty = TargetScope::TypeDef { name: "foo".into() };
        assert!(!overlaps(&func, &ty));
    }

    #[test]
    fn function_unscoped_overlaps_scoped_same_name() {
        let unscoped = TargetScope::Function {
            name: "render".into(),
            within: None,
        };
        let scoped = TargetScope::Function {
            name: "render".into(),
            within: Some(Box::new(TargetScope::ImplBlock {
                type_name: "Widget".into(),
                trait_name: None,
            })),
        };
        assert!(overlaps(&unscoped, &scoped));
    }

    #[test]
    fn scoped_functions_same_name_different_impl_disjoint() {
        let a = TargetScope::Function {
            name: "render".into(),
            within: Some(Box::new(TargetScope::ImplBlock {
                type_name: "Widget".into(),
                trait_name: None,
            })),
        };
        let b = TargetScope::Function {
            name: "render".into(),
            within: Some(Box::new(TargetScope::ImplBlock {
                type_name: "Button".into(),
                trait_name: None,
            })),
        };
        assert!(!overlaps(&a, &b));
    }

    #[test]
    fn impl_with_trait_overlaps_function_within_inherent() {
        let imp = TargetScope::ImplBlock {
            type_name: "Config".into(),
            trait_name: Some("Display".into()),
        };
        let func = TargetScope::Function {
            name: "fmt".into(),
            within: Some(Box::new(TargetScope::ImplBlock {
                type_name: "Config".into(),
                trait_name: None,
            })),
        };
        // Conservative: trait_name=None in parent means "any impl for Config"
        assert!(overlaps(&imp, &func));
    }
}
