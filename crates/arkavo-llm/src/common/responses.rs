//! Wire shapes shared by the two Responses-family providers (OpenAI, xAI).
//!
//! Both speak `POST /responses`, so the tool normalization and the usage
//! mapping are the same shape. What differs is each provider's tolerance for a
//! malformed report, and that policy stays with the provider that chose it.

use crate::provider::InferenceTiming;
use crate::{Error, Result};
use serde_json::{Value, json};

/// Normalize one tool declaration into a Responses `function` entry.
///
/// Callers hand this crate three shapes: Responses-native and OpenAI's nested
/// `{function: {...}}`, plus the router's Anthropic-style
/// `{name, description, input_schema}`. `None` means the value names no
/// function, which each provider answers with its own policy — an error where
/// a dropped tool would silently change the model's options, a skip where the
/// array may legitimately carry built-in tool types this crate does not model.
pub(crate) fn function_tool(tool: &Value) -> Option<Value> {
    let function = tool
        .get("function")
        .filter(|f| f.is_object())
        .unwrap_or(tool);
    let name = function
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())?;
    let parameters = function
        .get("parameters")
        .or_else(|| function.get("input_schema"))
        .cloned()
        .unwrap_or_else(|| json!({"type":"object","properties":{}}));
    Some(json!({
        "type": "function",
        "name": name,
        "description": function.get("description").and_then(Value::as_str).unwrap_or(""),
        "parameters": parameters,
    }))
}

/// A Responses `usage` block, as reported.
///
/// Details a provider omits stay `None` rather than becoming zero: "not
/// reported" and "none consumed" price the same but read differently.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Usage {
    pub(crate) input: u32,
    pub(crate) output: u32,
    pub(crate) cached: Option<u32>,
    pub(crate) cache_write: Option<u32>,
    pub(crate) reasoning: Option<u32>,
}

impl Usage {
    /// Read a `usage` object, rejecting counts that are not token counts.
    pub(crate) fn parse(usage: &Value) -> Result<Self> {
        let count = |value: &Value| -> Result<u32> {
            value
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| Error::Provider("Invalid Responses usage count".into()))
        };
        let optional = |pointer: &str| -> Result<Option<u32>> {
            usage.pointer(pointer).map(count).transpose()
        };
        Ok(Self {
            input: count(&usage["input_tokens"])?,
            output: count(&usage["output_tokens"])?,
            cached: optional("/input_tokens_details/cached_tokens")?,
            cache_write: optional("/input_tokens_details/cache_write_tokens")?,
            reasoning: optional("/output_tokens_details/reasoning_tokens")?,
        })
    }

    /// Whether the reported subsets fit inside the totals they belong to.
    ///
    /// Cached and cache-write reads are disjoint slices of the input; reasoning
    /// is a slice of the output. A report that breaks that is not a usage
    /// report this crate can price.
    pub(crate) fn is_consistent(&self) -> bool {
        let cached = self.cached.unwrap_or(0);
        cached <= self.input
            && self.cache_write.unwrap_or(0) <= self.input.saturating_sub(cached)
            && self.reasoning.unwrap_or(0) <= self.output
    }

    /// Keep each reported subset inside the total it belongs to.
    pub(crate) fn clamped(self) -> Self {
        let cached = self.cached.map(|cached| cached.min(self.input));
        Self {
            cached,
            cache_write: self
                .cache_write
                .map(|write| write.min(self.input.saturating_sub(cached.unwrap_or(0)))),
            reasoning: self.reasoning.map(|thinking| thinking.min(self.output)),
            ..self
        }
    }

    /// Split the report into the disjoint buckets the cost path sums.
    ///
    /// Both providers report `output_tokens` as the total generated, reasoning
    /// included, so visible output is the remainder. Timings stay zero: a
    /// Responses `usage` block carries no wall clock.
    pub(crate) fn timing(&self) -> InferenceTiming {
        InferenceTiming {
            n_prompt_eval: self.input,
            n_cached_prompt_eval: self.cached,
            n_cache_write_prompt_eval: self.cache_write,
            n_eval: self.output.saturating_sub(self.reasoning.unwrap_or(0)),
            n_thinking_eval: self.reasoning,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_and_bare_tool_shapes_normalize_to_one_entry() {
        let nested = function_tool(&json!({
            "type":"function",
            "function":{"name":"read","description":"read a file","parameters":{"type":"object","properties":{"path":{"type":"string"}}}}
        }))
        .expect("nested function tool");
        let bare = function_tool(&json!({
            "name":"read", "description":"read a file",
            "input_schema":{"type":"object","properties":{"path":{"type":"string"}}}
        }))
        .expect("router-shaped tool");
        assert_eq!(nested, bare);
        assert_eq!(bare["type"], "function");
        assert!(bare["parameters"]["properties"]["path"].is_object());
    }

    #[test]
    fn a_tool_without_a_usable_name_normalizes_to_nothing() {
        assert!(function_tool(&json!({"type":"function"})).is_none());
        assert!(function_tool(&json!({"name":""})).is_none());
        assert!(function_tool(&json!({"type":"web_search"})).is_none());
    }

    #[test]
    fn a_tool_without_parameters_declares_an_empty_object() {
        let tool = function_tool(&json!({"name":"now"})).expect("named tool");
        assert_eq!(tool["parameters"], json!({"type":"object","properties":{}}));
        assert_eq!(tool["description"], "");
    }

    #[test]
    fn usage_buckets_are_disjoint_so_cost_paths_do_not_double_count() {
        let usage = Usage::parse(&json!({
            "input_tokens":100, "output_tokens":60,
            "input_tokens_details":{"cached_tokens":75,"cache_write_tokens":10},
            "output_tokens_details":{"reasoning_tokens":40}
        }))
        .unwrap();
        assert!(usage.is_consistent());
        let timing = usage.timing();
        assert_eq!(timing.n_prompt_eval, 100);
        assert_eq!(timing.n_cached_prompt_eval, Some(75));
        assert_eq!(timing.n_cache_write_prompt_eval, Some(10));
        assert_eq!(timing.n_eval, 20);
        assert_eq!(timing.n_thinking_eval, Some(40));
        assert_eq!(
            timing.n_eval + timing.n_thinking_eval.unwrap(),
            usage.output
        );
    }

    #[test]
    fn absent_details_stay_unreported() {
        let usage = Usage::parse(&json!({"input_tokens":10,"output_tokens":5})).unwrap();
        let timing = usage.timing();
        assert_eq!(timing.n_eval, 5);
        assert_eq!(timing.n_thinking_eval, None);
        assert_eq!(timing.n_cached_prompt_eval, None);
        assert_eq!(timing.n_cache_write_prompt_eval, None);
    }

    #[test]
    fn a_subset_larger_than_its_total_is_inconsistent_and_clamps() {
        let usage = Usage::parse(&json!({
            "input_tokens":40, "output_tokens":5,
            "input_tokens_details":{"cached_tokens":100},
            "output_tokens_details":{"reasoning_tokens":9}
        }))
        .unwrap();
        assert!(!usage.is_consistent());
        let clamped = usage.clamped();
        assert_eq!(clamped.cached, Some(40));
        assert_eq!(clamped.reasoning, Some(5));
        assert_eq!(clamped.timing().n_eval, 0);
    }

    #[test]
    fn a_count_that_is_not_a_token_count_fails() {
        assert!(Usage::parse(&json!({"input_tokens":"many","output_tokens":1})).is_err());
        assert!(Usage::parse(&json!({"input_tokens":1,"output_tokens":-2})).is_err());
        assert!(
            Usage::parse(&json!({
                "input_tokens":1,"output_tokens":1,
                "output_tokens_details":{"reasoning_tokens":1.5}
            }))
            .is_err()
        );
    }
}
