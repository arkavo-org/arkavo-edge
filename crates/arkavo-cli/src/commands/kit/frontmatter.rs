//! Frontmatter `preflight:` / `kas:` / `budget:` → SwarmKit `runtime.*`
//! mapping for `kit migrate-from-agents-md` (finding 2).
//!
//! `legacy_agents_md::parse_legacy_agents_md`'s line-based parser only
//! understands a fixed set of top-level YAML keys (`KNOWN_SECTIONS`); any
//! other top-level key — including `preflight:`, `kas:`, and `budget:` —
//! is treated as an "unknown section" and every line inside it is skipped
//! outright, with no record that anything was dropped. That silently loses
//! real policy: preflight moderation rules, KAS trust roots, and spend
//! caps.
//!
//! This module extracts the same raw frontmatter block and deserializes
//! just those three keys directly into their SwarmKit runtime shapes.
//! `RuntimePreflight` and `RuntimeKas` were deliberately designed 1:1 with
//! the historical AGENTS.md frontmatter layout, so a direct `serde_yaml`
//! deserialize is sufficient — no field-by-field translation needed.

use arkavo_swarmkit::{CloudPolicyKind, RuntimeKas, RuntimePreflight};
use serde::Deserialize;

use super::legacy_agents_md_yaml::extract_frontmatter;

/// Runtime policy fields recovered from AGENTS.md frontmatter that the
/// legacy line-based parser has no representation for.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct FrontmatterRuntimeExtras {
    pub preflight: Option<RuntimePreflight>,
    pub kas: Option<RuntimeKas>,
    pub max_cost_per_session: Option<f64>,
    pub max_cost_per_day: Option<f64>,
    pub cloud_policy: Option<CloudPolicyKind>,
    /// One line per frontmatter key (`preflight`, `kas`, or `budget`) that
    /// was present but failed to parse into its runtime shape — malformed
    /// value or an unrecognized sub-key. Routed by the caller into the
    /// migrate command's existing `unmapped` → non-zero-exit mechanism
    /// rather than being silently dropped.
    pub parse_errors: Vec<String>,
}

/// Only the three keys this module owns; every other frontmatter key
/// (`name`, `purpose`, `model`, `a2a`, ...) is left for the line-based
/// parser and is simply ignored here — `serde_yaml` does not error on
/// unrecognized fields unless the struct opts into `deny_unknown_fields`.
#[derive(Debug, Deserialize, Default)]
struct RawExtras {
    #[serde(default)]
    preflight: Option<serde_yaml::Value>,
    #[serde(default)]
    kas: Option<serde_yaml::Value>,
    #[serde(default)]
    budget: Option<serde_yaml::Value>,
}

/// `budget:` has no historical frontmatter precedent (no shipped AGENTS.md
/// example ever used it — `KitRuntimeConfig`'s cost fields are flat, not
/// nested), so this shape is this migration's own design rather than a
/// reproduction of prior art: a `budget:` map with the same field names as
/// `KitRuntimeConfig`'s flat cost fields. `deny_unknown_fields` catches a
/// plausible-looking but wrong key (e.g. `max_per_session_usd` instead of
/// `max_cost_per_session`) as a parse error instead of silently no-op'ing.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBudget {
    #[serde(default)]
    max_cost_per_session: Option<f64>,
    #[serde(default)]
    max_cost_per_day: Option<f64>,
    #[serde(default)]
    cloud_policy: Option<CloudPolicyKind>,
}

/// Parse `preflight:`, `kas:`, and `budget:` out of `content`'s YAML
/// frontmatter block, if any. Returns `FrontmatterRuntimeExtras::default()`
/// (all `None`, no errors) when there is no frontmatter block at all —
/// that is the markdown-format AGENTS.md case, which never had these keys.
pub(super) fn extract_runtime_extras(content: &str) -> FrontmatterRuntimeExtras {
    let mut extras = FrontmatterRuntimeExtras::default();
    let Some(frontmatter) = extract_frontmatter(content) else {
        return extras;
    };

    let raw: RawExtras = match serde_yaml::from_str(frontmatter) {
        Ok(raw) => raw,
        Err(e) => {
            extras
                .parse_errors
                .push(format!("unmapped: frontmatter is not valid YAML ({e})"));
            return extras;
        }
    };

    if let Some(value) = raw.preflight {
        match serde_yaml::from_value::<RuntimePreflight>(value) {
            Ok(preflight) => extras.preflight = Some(preflight),
            Err(e) => extras
                .parse_errors
                .push(format!("unmapped: preflight ({e})")),
        }
    }

    if let Some(value) = raw.kas {
        match serde_yaml::from_value::<RuntimeKas>(value) {
            Ok(kas) => extras.kas = Some(kas),
            Err(e) => extras.parse_errors.push(format!("unmapped: kas ({e})")),
        }
    }

    if let Some(value) = raw.budget {
        match serde_yaml::from_value::<RawBudget>(value) {
            Ok(budget) => {
                extras.max_cost_per_session = budget.max_cost_per_session;
                extras.max_cost_per_day = budget.max_cost_per_day;
                extras.cloud_policy = budget.cloud_policy;
            }
            Err(e) => extras.parse_errors.push(format!("unmapped: budget ({e})")),
        }
    }

    extras
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_swarmkit::PreflightAction;

    #[test]
    fn no_frontmatter_returns_defaults() {
        let extras = extract_runtime_extras("## my-agent\nname: my-agent\n");
        assert_eq!(extras, FrontmatterRuntimeExtras::default());
    }

    #[test]
    fn maps_preflight_policies_1_to_1() {
        let content = r#"---
name: secure-agent
purpose: "demo"
model: ministral-3b

preflight:
  policies:
    - id: block_pii
      features:
        - InputContainsPII
      action: block
      description: "Blocks PII"
      enabled: true
    - id: block_sql_injection
      features:
        - InputContainsSQLKeywords
      action: block
      enabled: true
---
"#;
        let extras = extract_runtime_extras(content);
        assert!(extras.parse_errors.is_empty(), "{:?}", extras.parse_errors);
        let preflight = extras.preflight.expect("preflight must be mapped");
        assert_eq!(preflight.policies.len(), 2);
        assert_eq!(preflight.policies[0].id, "block_pii");
        assert_eq!(preflight.policies[0].features, vec!["InputContainsPII"]);
        assert_eq!(preflight.policies[0].action, PreflightAction::Block);
        assert_eq!(
            preflight.policies[0].description.as_deref(),
            Some("Blocks PII")
        );
    }

    #[test]
    fn maps_kas_with_trusted_roots() {
        let content = r#"---
name: kas-agent
purpose: "demo"
model: ministral-3b

kas:
  enabled: true
  key_id: "kas-demo-key-1"
  algorithm: "ec:secp256r1"
  trusted_roots:
    - did: "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
      name: "Demo Root Authority"
---
"#;
        let extras = extract_runtime_extras(content);
        assert!(extras.parse_errors.is_empty(), "{:?}", extras.parse_errors);
        let kas = extras.kas.expect("kas must be mapped");
        assert!(kas.enabled);
        assert_eq!(kas.key_id.as_deref(), Some("kas-demo-key-1"));
        assert_eq!(kas.algorithm.as_deref(), Some("ec:secp256r1"));
        assert_eq!(kas.trusted_roots.len(), 1);
        assert_eq!(
            kas.trusted_roots[0].did,
            "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
        );
    }

    #[test]
    fn maps_budget_cost_fields() {
        let content = r#"---
name: spend-agent
purpose: "demo"
model: ministral-3b

budget:
  max_cost_per_session: 1.5
  max_cost_per_day: 10.0
  cloud_policy: local_only
---
"#;
        let extras = extract_runtime_extras(content);
        assert!(extras.parse_errors.is_empty(), "{:?}", extras.parse_errors);
        assert_eq!(extras.max_cost_per_session, Some(1.5));
        assert_eq!(extras.max_cost_per_day, Some(10.0));
        assert_eq!(extras.cloud_policy, Some(CloudPolicyKind::LocalOnly));
    }

    #[test]
    fn unknown_budget_key_is_a_parse_error() {
        let content = r#"---
name: spend-agent
purpose: "demo"
model: ministral-3b

budget:
  max_per_session_usd: 1.5
---
"#;
        let extras = extract_runtime_extras(content);
        assert!(extras.max_cost_per_session.is_none());
        assert_eq!(extras.parse_errors.len(), 1);
        assert!(extras.parse_errors[0].contains("budget"));
    }

    #[test]
    fn malformed_preflight_is_a_parse_error() {
        let content = r#"---
name: broken-agent
purpose: "demo"
model: ministral-3b

preflight:
  policies: "this should be a list, not a string"
---
"#;
        let extras = extract_runtime_extras(content);
        assert!(extras.preflight.is_none());
        assert_eq!(extras.parse_errors.len(), 1);
        assert!(extras.parse_errors[0].contains("preflight"));
    }
}
