use proc_macro::TokenStream;
use quote::quote;
use syn::{LitStr, parse_macro_input, punctuated::Punctuated, token::Comma};

/// Attribute macro for spec-driven test traceability.
///
/// Validates spec IDs at compile time and emits `#[doc = "spec:ID"]` attributes
/// discoverable by xtask and external tools (TestRail, Xray, etc.).
///
/// # Examples
///
/// ```ignore
/// #[spec("HRM-003")]
/// #[test]
/// fn test_budget_enforcement() { /* ... */ }
///
/// #[spec("GOSSIP-005", "GOSSIP-006")]
/// #[tokio::test]
/// async fn test_anti_entropy() { /* ... */ }
/// ```
#[proc_macro_attribute]
pub fn spec(attr: TokenStream, item: TokenStream) -> TokenStream {
    let ids = parse_macro_input!(attr with Punctuated::<LitStr, Comma>::parse_terminated);

    if ids.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "spec() requires at least one ID",
        )
        .to_compile_error()
        .into();
    }

    let mut doc_attrs = Vec::new();
    for lit in &ids {
        let id = lit.value();
        if !is_valid_spec_id(&id) {
            return syn::Error::new(
                lit.span(),
                format!("invalid spec ID `{id}`: must match [A-Z]+-[0-9]+ (e.g. HRM-003)"),
            )
            .to_compile_error()
            .into();
        }
        let doc_value = format!("spec:{id}");
        doc_attrs.push(quote! { #[doc = #doc_value] });
    }

    let item = proc_macro2::TokenStream::from(item);
    let expanded = quote! {
        #(#doc_attrs)*
        #item
    };
    expanded.into()
}

fn is_valid_spec_id(id: &str) -> bool {
    let Some((prefix, num)) = id.split_once('-') else {
        return false;
    };
    !prefix.is_empty()
        && prefix.chars().all(|c| c.is_ascii_uppercase())
        && !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_spec_ids() {
        assert!(is_valid_spec_id("HRM-003"));
        assert!(is_valid_spec_id("GOSSIP-005"));
        assert!(is_valid_spec_id("EVENT-005"));
        assert!(is_valid_spec_id("CRIT-011"));
        assert!(is_valid_spec_id("A-1"));
    }

    #[test]
    fn test_invalid_spec_ids() {
        assert!(!is_valid_spec_id("hrm-003"));
        assert!(!is_valid_spec_id("HRM003"));
        assert!(!is_valid_spec_id("HRM-"));
        assert!(!is_valid_spec_id("-003"));
        assert!(!is_valid_spec_id(""));
        assert!(!is_valid_spec_id("HRM-00a"));
        assert!(!is_valid_spec_id("Hrm-003"));
    }
}
