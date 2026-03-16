use crate::ast_op::AstOp;
use crate::error::{Error, Result};
use crate::op_bundle::OpBundle;
use crate::target_scope::TargetScope;

/// Applies typed AST operations to Rust source code via `syn`.
pub struct RustOp;

impl RustOp {
    /// Parse Rust source code into a `syn::File`.
    pub fn parse(source: &str) -> Result<syn::File> {
        syn::parse_str(source).map_err(|e| Error::Parse(e.to_string()))
    }

    /// Apply all operations from an `OpBundle` to source code.
    /// Returns the modified AST. Fails the entire bundle on any error.
    pub fn apply_bundle(source: &str, bundle: &OpBundle) -> Result<syn::File> {
        let mut file = Self::parse(source)?;
        for op in &bundle.ops {
            Self::apply(&mut file, op)?;
        }
        Ok(file)
    }

    /// Apply a single `AstOp` to a parsed file.
    pub fn apply(file: &mut syn::File, op: &AstOp) -> Result<()> {
        match op {
            AstOp::ReplaceFnBody { scope, new_body } => replace_fn_body(file, scope, new_body),
            AstOp::InsertAfter { scope, new_item } => insert_after(file, scope, new_item),
            AstOp::Remove { scope } => remove_item(file, scope),
            AstOp::ReplaceItem { scope, new_item } => replace_item(file, scope, new_item),
            AstOp::AddAttribute { scope, attribute } => add_attribute(file, scope, attribute),
            AstOp::AddUse { path } => add_use(file, path),
        }
    }
}

fn find_unique_item_index(file: &syn::File, scope: &TargetScope) -> Result<usize> {
    let matches: Vec<usize> = file
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| scope.matches_item(item))
        .map(|(i, _)| i)
        .collect();

    match matches.len() {
        0 => Err(Error::TargetNotFound(scope.to_string())),
        1 => Ok(matches[0]),
        n => Err(Error::AmbiguousTarget(n, scope.to_string())),
    }
}

fn replace_fn_body(file: &mut syn::File, scope: &TargetScope, new_body: &str) -> Result<()> {
    let block: syn::Block =
        syn::parse_str(new_body).map_err(|e| Error::Parse(format!("new body: {e}")))?;

    if scope.is_scoped_function() {
        return replace_method_body(file, scope, block);
    }

    let idx = find_unique_item_index(file, scope)?;
    match &mut file.items[idx] {
        syn::Item::Fn(f) => {
            *f.block = block;
            Ok(())
        }
        _ => Err(Error::TargetNotFound(format!("{scope} is not a function"))),
    }
}

fn replace_method_body(file: &mut syn::File, scope: &TargetScope, block: syn::Block) -> Result<()> {
    // Two-pass: first find location, then mutate
    let location = find_method_location(file, scope)?;
    if let syn::Item::Impl(imp) = &mut file.items[location.0]
        && let syn::ImplItem::Fn(method) = &mut imp.items[location.1]
    {
        method.block = block;
        return Ok(());
    }
    Err(Error::TargetNotFound(scope.to_string()))
}

fn insert_after(file: &mut syn::File, scope: &TargetScope, new_item: &str) -> Result<()> {
    let item: syn::Item =
        syn::parse_str(new_item).map_err(|e| Error::Parse(format!("new item: {e}")))?;

    if matches!(scope, TargetScope::File) {
        file.items.push(item);
        return Ok(());
    }

    let idx = find_unique_item_index(file, scope)?;
    file.items.insert(idx + 1, item);
    Ok(())
}

fn remove_item(file: &mut syn::File, scope: &TargetScope) -> Result<()> {
    let idx = find_unique_item_index(file, scope)?;
    file.items.remove(idx);
    Ok(())
}

fn replace_item(file: &mut syn::File, scope: &TargetScope, new_item: &str) -> Result<()> {
    let item: syn::Item =
        syn::parse_str(new_item).map_err(|e| Error::Parse(format!("new item: {e}")))?;
    let idx = find_unique_item_index(file, scope)?;
    file.items[idx] = item;
    Ok(())
}

/// Find (impl_index, method_index) for a scoped function target.
fn find_method_location(file: &syn::File, scope: &TargetScope) -> Result<(usize, usize)> {
    for (impl_idx, item) in file.items.iter().enumerate() {
        if let syn::Item::Impl(imp) = item {
            for (method_idx, impl_item) in imp.items.iter().enumerate() {
                if let syn::ImplItem::Fn(method) = impl_item
                    && scope.matches_impl_method(&method.sig.ident.to_string(), imp)
                {
                    return Ok((impl_idx, method_idx));
                }
            }
        }
    }
    Err(Error::TargetNotFound(scope.to_string()))
}

fn parse_outer_attr(attribute: &str) -> Result<syn::Attribute> {
    // Parse as an item with the attribute, then extract the attribute
    let code = format!("{attribute} fn _dummy() {{}}");
    let item: syn::ItemFn =
        syn::parse_str(&code).map_err(|e| Error::Parse(format!("attribute `{attribute}`: {e}")))?;
    item.attrs
        .into_iter()
        .next()
        .ok_or_else(|| Error::Parse(format!("no attribute parsed from `{attribute}`")))
}

fn add_attribute(file: &mut syn::File, scope: &TargetScope, attribute: &str) -> Result<()> {
    let attr = parse_outer_attr(attribute)?;

    if scope.is_scoped_function() {
        return add_method_attribute(file, scope, attr);
    }

    let idx = find_unique_item_index(file, scope)?;
    match &mut file.items[idx] {
        syn::Item::Fn(f) => f.attrs.push(attr),
        syn::Item::Struct(s) => s.attrs.push(attr),
        syn::Item::Enum(e) => e.attrs.push(attr),
        syn::Item::Impl(i) => i.attrs.push(attr),
        syn::Item::Type(t) => t.attrs.push(attr),
        _ => return Err(Error::Parse(format!("cannot add attribute to {scope}"))),
    }
    Ok(())
}

fn add_method_attribute(
    file: &mut syn::File,
    scope: &TargetScope,
    attr: syn::Attribute,
) -> Result<()> {
    let location = find_method_location(file, scope)?;
    if let syn::Item::Impl(imp) = &mut file.items[location.0]
        && let syn::ImplItem::Fn(method) = &mut imp.items[location.1]
    {
        method.attrs.push(attr);
        return Ok(());
    }
    Err(Error::TargetNotFound(scope.to_string()))
}

fn add_use(file: &mut syn::File, path: &str) -> Result<()> {
    let use_item: syn::ItemUse = syn::parse_str(&format!("use {path};"))
        .map_err(|e| Error::Parse(format!("use path `{path}`: {e}")))?;

    // Insert after last existing use statement, or at the top
    let last_use = file
        .items
        .iter()
        .rposition(|item| matches!(item, syn::Item::Use(_)));

    let insert_pos = last_use.map_or(0, |i| i + 1);
    file.items.insert(insert_pos, syn::Item::Use(use_item));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::render_file;

    fn apply_and_render(source: &str, op: AstOp) -> String {
        let mut file = RustOp::parse(source).unwrap();
        RustOp::apply(&mut file, &op).unwrap();
        render_file(&file)
    }

    #[test]
    fn replace_free_fn_body() {
        let source = "fn foo() -> i32 { 1 }";
        let result = apply_and_render(
            source,
            AstOp::ReplaceFnBody {
                scope: TargetScope::Function {
                    name: "foo".into(),
                    within: None,
                },
                new_body: "{ 42 }".into(),
            },
        );
        assert!(result.contains("42"));
        assert!(!result.contains("1"));
    }

    #[test]
    fn replace_method_body_in_impl() {
        let source = r#"
            impl Foo {
                fn bar(&self) -> i32 { 1 }
                fn baz(&self) -> i32 { 2 }
            }
        "#;
        let result = apply_and_render(
            source,
            AstOp::ReplaceFnBody {
                scope: TargetScope::Function {
                    name: "bar".into(),
                    within: Some(Box::new(TargetScope::ImplBlock {
                        type_name: "Foo".into(),
                        trait_name: None,
                    })),
                },
                new_body: "{ 99 }".into(),
            },
        );
        assert!(result.contains("99"));
        assert!(result.contains("2")); // baz unchanged
    }

    #[test]
    fn insert_after_function() {
        let source = "fn first() {} fn second() {}";
        let result = apply_and_render(
            source,
            AstOp::InsertAfter {
                scope: TargetScope::Function {
                    name: "first".into(),
                    within: None,
                },
                new_item: "fn middle() {}".into(),
            },
        );
        let first_pos = result.find("first").unwrap();
        let middle_pos = result.find("middle").unwrap();
        let second_pos = result.find("second").unwrap();
        assert!(first_pos < middle_pos);
        assert!(middle_pos < second_pos);
    }

    #[test]
    fn insert_at_file_end() {
        let source = "fn existing() {}";
        let result = apply_and_render(
            source,
            AstOp::InsertAfter {
                scope: TargetScope::File,
                new_item: "fn appended() {}".into(),
            },
        );
        assert!(result.contains("appended"));
    }

    #[test]
    fn remove_function() {
        let source = "fn keep() {} fn remove_me() {} fn also_keep() {}";
        let result = apply_and_render(
            source,
            AstOp::Remove {
                scope: TargetScope::Function {
                    name: "remove_me".into(),
                    within: None,
                },
            },
        );
        assert!(result.contains("keep"));
        assert!(!result.contains("remove_me"));
        assert!(result.contains("also_keep"));
    }

    #[test]
    fn replace_item() {
        let source = "fn old() -> i32 { 1 }";
        let result = apply_and_render(
            source,
            AstOp::ReplaceItem {
                scope: TargetScope::Function {
                    name: "old".into(),
                    within: None,
                },
                new_item: "fn new_fn() -> String { String::new() }".into(),
            },
        );
        assert!(!result.contains("old"));
        assert!(result.contains("new_fn"));
    }

    #[test]
    fn add_attribute_to_function() {
        let source = "fn foo() {}";
        let result = apply_and_render(
            source,
            AstOp::AddAttribute {
                scope: TargetScope::Function {
                    name: "foo".into(),
                    within: None,
                },
                attribute: "#[inline]".into(),
            },
        );
        assert!(result.contains("#[inline]"));
        assert!(result.contains("fn foo"));
    }

    #[test]
    fn add_attribute_to_method() {
        let source = "impl Foo { fn bar(&self) {} }";
        let result = apply_and_render(
            source,
            AstOp::AddAttribute {
                scope: TargetScope::Function {
                    name: "bar".into(),
                    within: Some(Box::new(TargetScope::ImplBlock {
                        type_name: "Foo".into(),
                        trait_name: None,
                    })),
                },
                attribute: "#[must_use]".into(),
            },
        );
        assert!(result.contains("#[must_use]"));
    }

    #[test]
    fn add_use_statement() {
        let source = "use std::io;\nfn foo() {}";
        let result = apply_and_render(
            source,
            AstOp::AddUse {
                path: "std::collections::HashMap".into(),
            },
        );
        assert!(result.contains("use std::collections::HashMap;"));
        // New use should appear after existing use
        let io_pos = result.find("std::io").unwrap();
        let hashmap_pos = result.find("HashMap").unwrap();
        assert!(io_pos < hashmap_pos);
    }

    #[test]
    fn add_use_to_empty_file() {
        let source = "fn foo() {}";
        let result = apply_and_render(
            source,
            AstOp::AddUse {
                path: "std::io".into(),
            },
        );
        assert!(result.contains("use std::io;"));
        // Use should appear before fn
        let use_pos = result.find("use").unwrap();
        let fn_pos = result.find("fn").unwrap();
        assert!(use_pos < fn_pos);
    }

    #[test]
    fn target_not_found() {
        let mut file = RustOp::parse("fn foo() {}").unwrap();
        let result = RustOp::apply(
            &mut file,
            &AstOp::Remove {
                scope: TargetScope::Function {
                    name: "nonexistent".into(),
                    within: None,
                },
            },
        );
        assert!(matches!(result, Err(Error::TargetNotFound(_))));
    }

    #[test]
    fn ambiguous_target() {
        let source = "fn dup() {} fn dup() {}";
        let mut file = RustOp::parse(source).unwrap();
        let result = RustOp::apply(
            &mut file,
            &AstOp::Remove {
                scope: TargetScope::Function {
                    name: "dup".into(),
                    within: None,
                },
            },
        );
        assert!(matches!(result, Err(Error::AmbiguousTarget(2, _))));
    }

    #[test]
    fn invalid_new_body() {
        let mut file = RustOp::parse("fn foo() {}").unwrap();
        let result = RustOp::apply(
            &mut file,
            &AstOp::ReplaceFnBody {
                scope: TargetScope::Function {
                    name: "foo".into(),
                    within: None,
                },
                new_body: "{ not valid rust @@@ }".into(),
            },
        );
        assert!(matches!(result, Err(Error::Parse(_))));
    }

    #[test]
    fn bundle_applies_multiple_ops() {
        let source = "fn foo() -> i32 { 1 }\nfn bar() -> i32 { 2 }";
        let bundle = OpBundle::new(
            "test".into(),
            "test.rs".into(),
            vec![
                AstOp::ReplaceFnBody {
                    scope: TargetScope::Function {
                        name: "foo".into(),
                        within: None,
                    },
                    new_body: "{ 10 }".into(),
                },
                AstOp::AddAttribute {
                    scope: TargetScope::Function {
                        name: "bar".into(),
                        within: None,
                    },
                    attribute: "#[inline]".into(),
                },
            ],
            "optimize".into(),
        );
        let result = RustOp::apply_bundle(source, &bundle).unwrap();
        let rendered = render_file(&result);
        assert!(rendered.contains("10"));
        assert!(rendered.contains("#[inline]"));
    }

    #[test]
    fn bundle_fails_atomically() {
        let source = "fn foo() {} fn bar() {}";
        let bundle = OpBundle::new(
            "test".into(),
            "test.rs".into(),
            vec![
                AstOp::Remove {
                    scope: TargetScope::Function {
                        name: "foo".into(),
                        within: None,
                    },
                },
                AstOp::Remove {
                    scope: TargetScope::Function {
                        name: "nonexistent".into(),
                        within: None,
                    },
                },
            ],
            "cleanup".into(),
        );
        assert!(RustOp::apply_bundle(source, &bundle).is_err());
    }

    #[test]
    fn roundtrip_parse_render_parse() {
        let source = r#"
use std::io;

struct Config {
    name: String,
    value: u64,
}

impl Config {
    fn new(name: String) -> Self {
        Self { name, value: 0 }
    }
}

fn process(cfg: &Config) -> bool {
    cfg.value > 0
}
"#;
        let file = RustOp::parse(source).unwrap();
        let rendered = render_file(&file);
        let reparsed = RustOp::parse(&rendered).unwrap();
        let re_rendered = render_file(&reparsed);
        assert_eq!(rendered, re_rendered);
    }
}
