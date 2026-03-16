use serde::{Deserialize, Serialize};

/// Semantic targeting for AST nodes.
///
/// Operations target nodes by semantic identity (name, type) rather than
/// position, making most operations on disjoint subtrees naturally commutative.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum TargetScope {
    /// Target a function by name, optionally scoped to a parent impl block
    Function {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        within: Option<Box<TargetScope>>,
    },
    /// Target an impl block by type name and optional trait
    ImplBlock {
        type_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trait_name: Option<String>,
    },
    /// Target a struct or enum definition by name
    TypeDef { name: String },
    /// Target a use statement by path
    UseStatement { path: String },
    /// Target the entire file
    File,
}

impl std::fmt::Display for TargetScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Function { name, within: None } => write!(f, "fn {name}"),
            Self::Function {
                name,
                within: Some(parent),
            } => write!(f, "{parent}::fn {name}"),
            Self::ImplBlock {
                type_name,
                trait_name: None,
            } => write!(f, "impl {type_name}"),
            Self::ImplBlock {
                type_name,
                trait_name: Some(t),
            } => write!(f, "impl {t} for {type_name}"),
            Self::TypeDef { name } => write!(f, "type {name}"),
            Self::UseStatement { path } => write!(f, "use {path}"),
            Self::File => write!(f, "<file>"),
        }
    }
}

impl TargetScope {
    /// Check if a top-level `syn::Item` matches this scope.
    pub fn matches_item(&self, item: &syn::Item) -> bool {
        match self {
            Self::Function { name, within: None } => {
                matches!(item, syn::Item::Fn(f) if f.sig.ident == name)
            }
            Self::ImplBlock {
                type_name,
                trait_name,
            } => {
                matches!(item, syn::Item::Impl(imp) if impl_matches(imp, type_name, trait_name.as_deref()))
            }
            Self::TypeDef { name } => match item {
                syn::Item::Struct(s) => s.ident == name,
                syn::Item::Enum(e) => e.ident == name,
                syn::Item::Type(t) => t.ident == name,
                _ => false,
            },
            Self::UseStatement { path } => {
                matches!(item, syn::Item::Use(u) if use_path_contains(u, path))
            }
            Self::File => true,
            // Function with `within` doesn't match top-level items directly;
            // it requires walking into the parent impl block.
            Self::Function {
                within: Some(_), ..
            } => false,
        }
    }

    /// Check if a method inside an impl block matches this scope.
    /// Only meaningful for `Function { within: Some(...) }`.
    pub fn matches_impl_method(&self, method_name: &str, parent_impl: &syn::ItemImpl) -> bool {
        match self {
            Self::Function {
                name,
                within: Some(parent_scope),
            } => name == method_name && impl_matches_scope(parent_impl, parent_scope),
            _ => false,
        }
    }

    /// Whether this scope targets a method inside an impl block.
    pub fn is_scoped_function(&self) -> bool {
        matches!(
            self,
            Self::Function {
                within: Some(_),
                ..
            }
        )
    }
}

fn impl_matches(imp: &syn::ItemImpl, type_name: &str, trait_name: Option<&str>) -> bool {
    let ty_matches = match imp.self_ty.as_ref() {
        syn::Type::Path(tp) => tp
            .path
            .segments
            .last()
            .is_some_and(|seg| seg.ident == type_name),
        _ => false,
    };
    if !ty_matches {
        return false;
    }
    match (trait_name, &imp.trait_) {
        (None, None) => true,
        (Some(expected), Some((_, path, _))) => path
            .segments
            .last()
            .is_some_and(|seg| seg.ident == expected),
        _ => false,
    }
}

fn impl_matches_scope(imp: &syn::ItemImpl, scope: &TargetScope) -> bool {
    match scope {
        TargetScope::ImplBlock {
            type_name,
            trait_name,
        } => impl_matches(imp, type_name, trait_name.as_deref()),
        _ => false,
    }
}

fn use_path_contains(use_item: &syn::ItemUse, path: &str) -> bool {
    let tokens = quote::quote!(#use_item);
    let rendered = tokens.to_string();
    rendered.contains(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_items(code: &str) -> Vec<syn::Item> {
        let file: syn::File = syn::parse_str(code).unwrap();
        file.items
    }

    #[test]
    fn matches_free_function() {
        let items = parse_items("fn foo() {} fn bar() {}");
        let scope = TargetScope::Function {
            name: "foo".into(),
            within: None,
        };
        assert!(scope.matches_item(&items[0]));
        assert!(!scope.matches_item(&items[1]));
    }

    #[test]
    fn matches_struct() {
        let items = parse_items("struct Foo; struct Bar;");
        let scope = TargetScope::TypeDef { name: "Foo".into() };
        assert!(scope.matches_item(&items[0]));
        assert!(!scope.matches_item(&items[1]));
    }

    #[test]
    fn matches_enum() {
        let items = parse_items("enum Color { Red, Blue }");
        let scope = TargetScope::TypeDef {
            name: "Color".into(),
        };
        assert!(scope.matches_item(&items[0]));
    }

    #[test]
    fn matches_inherent_impl() {
        let items = parse_items("impl Foo { fn bar() {} }");
        let scope = TargetScope::ImplBlock {
            type_name: "Foo".into(),
            trait_name: None,
        };
        assert!(scope.matches_item(&items[0]));
    }

    #[test]
    fn matches_trait_impl() {
        let items = parse_items(
            "impl Display for Foo { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { Ok(()) } }",
        );
        let scope = TargetScope::ImplBlock {
            type_name: "Foo".into(),
            trait_name: Some("Display".into()),
        };
        assert!(scope.matches_item(&items[0]));

        let wrong_trait = TargetScope::ImplBlock {
            type_name: "Foo".into(),
            trait_name: Some("Debug".into()),
        };
        assert!(!wrong_trait.matches_item(&items[0]));
    }

    #[test]
    fn matches_use_statement() {
        let items = parse_items("use std::collections::HashMap;");
        let scope = TargetScope::UseStatement {
            path: "HashMap".into(),
        };
        assert!(scope.matches_item(&items[0]));
    }

    #[test]
    fn file_matches_everything() {
        let items = parse_items("fn foo() {} struct Bar;");
        let scope = TargetScope::File;
        assert!(items.iter().all(|i| scope.matches_item(i)));
    }

    #[test]
    fn scoped_function_does_not_match_toplevel() {
        let items = parse_items("fn foo() {}");
        let scope = TargetScope::Function {
            name: "foo".into(),
            within: Some(Box::new(TargetScope::ImplBlock {
                type_name: "Bar".into(),
                trait_name: None,
            })),
        };
        assert!(!scope.matches_item(&items[0]));
    }

    #[test]
    fn matches_impl_method_with_scope() {
        let file: syn::File = syn::parse_str("impl Foo { fn bar(&self) -> i32 { 42 } }").unwrap();
        let imp = match &file.items[0] {
            syn::Item::Impl(imp) => imp,
            _ => panic!("expected impl"),
        };

        let scope = TargetScope::Function {
            name: "bar".into(),
            within: Some(Box::new(TargetScope::ImplBlock {
                type_name: "Foo".into(),
                trait_name: None,
            })),
        };
        assert!(scope.matches_impl_method("bar", imp));

        let wrong_scope = TargetScope::Function {
            name: "bar".into(),
            within: Some(Box::new(TargetScope::ImplBlock {
                type_name: "Baz".into(),
                trait_name: None,
            })),
        };
        assert!(!wrong_scope.matches_impl_method("bar", imp));
    }

    #[test]
    fn display_formatting() {
        assert_eq!(
            TargetScope::Function {
                name: "foo".into(),
                within: None
            }
            .to_string(),
            "fn foo"
        );
        assert_eq!(
            TargetScope::ImplBlock {
                type_name: "Foo".into(),
                trait_name: Some("Bar".into())
            }
            .to_string(),
            "impl Bar for Foo"
        );
    }

    #[test]
    fn serde_roundtrip() {
        let scope = TargetScope::Function {
            name: "test".into(),
            within: Some(Box::new(TargetScope::ImplBlock {
                type_name: "MyType".into(),
                trait_name: None,
            })),
        };
        let json = serde_json::to_string(&scope).unwrap();
        let back: TargetScope = serde_json::from_str(&json).unwrap();
        assert_eq!(scope, back);
    }
}
