//! `arkavo kit validate`: decide whether a kit file can be run.
//!
//! Structural validity is not enough to start an agent: `arkavo agent -c`
//! also refuses a kit that has expired or whose roles name a model the
//! router does not know. This module applies those same checks so a kit
//! that validates here is one the agent start path accepts.

use std::path::Path;

use arkavo_swarmkit::unenforced::AGENT_PATH;
use arkavo_swarmkit::{
    AgentRuntimeConfig, DiscoverError, UnenforcedControl, kit_id_for, read_kit_file,
    unenforced_on_agent_path, validate_not_expired,
};
use chrono::{DateTime, Utc};

use super::model_map::kit_model_to_hint;

/// Result of a successful `kit validate`.
#[derive(Debug)]
pub struct KitValidateReport {
    pub kit_name: String,
    /// The `kit.id` the file declares; empty while the kit is being authored.
    pub kit_id: String,
    /// The id recomputed from the manifest content.
    pub computed_id: String,
    /// False only for an unassigned (empty) `kit.id`; a non-empty id that
    /// does not match is an error, not a report.
    pub id_matches: bool,
    pub expires: Option<String>,
    /// Controls the kit declares that the agent path does not act on. They
    /// do not make the kit invalid.
    pub unenforced: Vec<UnenforcedControl>,
}

impl KitValidateReport {
    /// The lines `kit validate` prints for a valid kit.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!("kit: {}", self.kit_name)];
        if self.id_matches {
            lines.push(format!("kit.id: {}", self.kit_id));
            lines.push("kit.id matches recomputed hash: true".to_string());
        } else {
            lines.push("kit.id: not set".to_string());
            lines.push(format!("computed kit.id: {}", self.computed_id));
            lines.push(
                "Set kit.id to the computed value once the kit is final; \
                 any later edit changes the id."
                    .to_string(),
            );
        }
        match &self.expires {
            Some(expires) => lines.push(format!("expires: {expires}")),
            None => lines.push("expires: never".to_string()),
        }
        lines.extend(self.unenforced_notice());
        lines
    }

    /// A notice, not an error: the kit runs, but an author who set these
    /// fields should know they restrict nothing on this path.
    fn unenforced_notice(&self) -> Vec<String> {
        if self.unenforced.is_empty() {
            return Vec::new();
        }
        let mut lines = vec![
            String::new(),
            format!(
                "Notice: this kit is valid. It declares settings that '{AGENT_PATH}' does not enforce:"
            ),
        ];
        for control in &self.unenforced {
            if control.roles.is_empty() {
                lines.push(format!("  {}", control.field));
            } else {
                lines.push(format!(
                    "  {} (roles: {})",
                    control.field,
                    control.roles.join(", ")
                ));
            }
            lines.push(format!("      {}", control.effect));
        }
        lines
    }
}

/// Validate a kit file against the system clock.
pub fn validate_kit(path: &Path) -> Result<KitValidateReport, Box<dyn std::error::Error>> {
    validate_kit_at(path, Utc::now())
}

/// Validate a kit file as of `now`.
///
/// A kit that parses is checked for expiry and for unresolvable models
/// together, so one run reports everything that would stop it from starting.
pub fn validate_kit_at(
    path: &Path,
    now: DateTime<Utc>,
) -> Result<KitValidateReport, Box<dyn std::error::Error>> {
    let discovered = read_kit_file(path).map_err(describe_load_error)?;
    let manifest = &discovered.manifest;

    let mut problems = Vec::new();
    if let Err(expired) = validate_not_expired(manifest, now) {
        problems.push(expired.to_string());
    }
    problems.extend(unresolved_models(&discovered.config));
    if !problems.is_empty() {
        return Err(format!("{}: {}", path.display(), problems.join("; ")).into());
    }

    // `read_kit_file` has already rejected a non-empty id that does not
    // match, so the only way to differ here is an id that was never set.
    let computed_id = kit_id_for(manifest)?;
    Ok(KitValidateReport {
        kit_name: manifest.kit.name.clone(),
        kit_id: manifest.kit.id.clone(),
        id_matches: computed_id == manifest.kit.id,
        computed_id,
        expires: manifest.kit.expires.clone(),
        unenforced: unenforced_on_agent_path(manifest),
    })
}

/// A missing file is the most common mistake, and the loader's
/// `read <path>: <os error>` reads like an internal failure; say what was
/// expected instead.
fn describe_load_error(err: DiscoverError) -> Box<dyn std::error::Error> {
    match err {
        DiscoverError::Io { path, source } if source.kind() == std::io::ErrorKind::NotFound => {
            format!("kit file not found: {}", path.display()).into()
        }
        other => other.into(),
    }
}

/// One message per role whose declared model the router does not know.
///
/// Resolution is [`kit_model_to_hint`], the function the agent start path
/// uses, so the two cannot disagree about what is runnable. A role without
/// a model is fine: the router chooses.
pub(super) fn unresolved_models(config: &AgentRuntimeConfig) -> Vec<String> {
    config
        .roles
        .iter()
        .filter_map(|role| {
            let family = role.model_family.as_deref()?;
            let size = role.model_size.as_deref();
            if kit_model_to_hint(family, size).is_some() {
                return None;
            }
            let declared = match size {
                Some(size) => format!("family {family:?}, size {size:?}"),
                None => format!("family {family:?}"),
            };
            Some(format!(
                "role {:?}: model ({declared}) is not a model the router knows; name a local \
                 edge model as family/size (e.g. ministral/3B) or a router model id as the \
                 family with no size (e.g. gpt-6-astra)",
                role.role_id
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn at(raw: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(raw)
            .unwrap()
            .with_timezone(&Utc)
    }

    const VALID_ON: &str = "2026-05-01T00:00:00Z";

    /// A two-role kit with a fixed validity window; `id` and the second
    /// role's model are the parts each test varies.
    fn kit_yaml(id: &str, critic_model: &str) -> String {
        format!(
            r#"
spec_version: "1.0.0"
kit:
  id: "{id}"
  name: "fixture"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  expires: "2026-05-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "review"
roles:
  - id: planner
    role_type: planner
    agent_provisioning:
      model: {{family: ministral, size: 3B}}
  - id: critic
    role_type: critic
    agent_provisioning:
      model: {critic_model}
coordination:
  topology: pipeline
  protocol: a2a-jsonrpc-2.0
  routing:
    strategy: static
constraints:
  global_budget:
    max_wallclock_seconds: 60
    max_total_tokens: 8000
    max_cost_usd: 0.01
  network:
    egress_allowed: false
completion:
  rules: ["done"]
  on_failure: abort
  max_retries: 0
provenance:
  signatures: []
"#
        )
    }

    fn write_kit(dir: &tempfile::TempDir, yaml: &str) -> PathBuf {
        let path = dir.path().join("fixture.swarmkit.yaml");
        std::fs::write(&path, yaml).unwrap();
        path
    }

    const GOOD_MODEL: &str = "{family: gemma, size: 12B}";

    /// Regression: `kit validate` accepted a kit months past `kit.expires`.
    #[test]
    fn expired_kit_fails_with_its_expiry_date() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_kit(&dir, &kit_yaml("", GOOD_MODEL));

        validate_kit_at(&path, at(VALID_ON)).expect("valid inside its window");

        let err = validate_kit_at(&path, at("2026-09-29T00:00:00Z"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("expired on 2026-05-29T00:00:00Z"), "{err}");
        assert!(err.contains("fixture.swarmkit.yaml"), "{err}");
    }

    /// Regression: `kit validate` passed a kit that `arkavo agent -c` then
    /// refused to start because a role named a model the router lacks.
    #[test]
    fn unresolvable_model_fails_naming_the_role_and_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_kit(&dir, &kit_yaml("", "{family: gemma-4, size: 9B}"));

        let err = validate_kit_at(&path, at(VALID_ON))
            .unwrap_err()
            .to_string();
        assert!(err.contains("role \"critic\""), "{err}");
        assert!(err.contains("\"gemma-4\""), "{err}");
        assert!(err.contains("\"9B\""), "{err}");
        assert!(
            !err.contains("role \"planner\""),
            "a role with a known model must not be reported: {err}"
        );
    }

    #[test]
    fn expiry_and_model_problems_are_reported_together() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_kit(&dir, &kit_yaml("", "{family: qwen3, size: 7B}"));

        let err = validate_kit_at(&path, at("2026-09-29T00:00:00Z"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("expired on"), "{err}");
        assert!(err.contains("\"qwen3\""), "{err}");
    }

    /// Regression: the documented authoring state `kit.id: ""` failed with
    /// `kit.id mismatch: declared "", recomputed "blake3:..."`.
    #[test]
    fn empty_kit_id_is_valid_and_reports_the_computed_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_kit(&dir, &kit_yaml("", GOOD_MODEL));

        let report = validate_kit_at(&path, at(VALID_ON)).expect("an unset id is not an error");
        assert_eq!(report.kit_id, "");
        assert!(!report.id_matches);
        assert!(report.computed_id.starts_with("blake3:"));

        let printed = report.lines().join("\n");
        assert!(printed.contains("kit.id: not set"), "{printed}");
        assert!(
            printed.contains(&format!("computed kit.id: {}", report.computed_id)),
            "{printed}"
        );
        assert!(
            printed.contains("Set kit.id to the computed value"),
            "{printed}"
        );
    }

    #[test]
    fn computed_id_round_trips_into_a_matching_kit() {
        let dir = tempfile::tempdir().unwrap();
        let unset = write_kit(&dir, &kit_yaml("", GOOD_MODEL));
        let computed = validate_kit_at(&unset, at(VALID_ON)).unwrap().computed_id;

        let assigned = write_kit(&dir, &kit_yaml(&computed, GOOD_MODEL));
        let report = validate_kit_at(&assigned, at(VALID_ON)).unwrap();
        assert!(report.id_matches);
        assert_eq!(report.kit_id, computed);
        assert!(
            report
                .lines()
                .contains(&"kit.id matches recomputed hash: true".to_string())
        );
    }

    #[test]
    fn non_empty_kit_id_that_does_not_match_still_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_kit(&dir, &kit_yaml("blake3:not-the-real-hash", GOOD_MODEL));

        let err = validate_kit_at(&path, at(VALID_ON))
            .unwrap_err()
            .to_string();
        assert!(err.contains("blake3:not-the-real-hash"), "{err}");
        assert!(err.contains("does not match"), "{err}");
    }

    /// Regression: `kit validate` said nothing about declared limits that
    /// the agent path ignores, so a valid kit read as an enforced one.
    #[test]
    fn unenforced_controls_are_printed_as_a_notice_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = kit_yaml(
            "",
            "{family: gemma, size: 12B}\n      isolation: {sandbox: process, network_egress: false}",
        );
        let path = write_kit(&dir, &yaml);

        let report = validate_kit_at(&path, at(VALID_ON)).expect("the notice is not a failure");
        let lines = report.lines();
        let notice_at = lines
            .iter()
            .position(|l| l.starts_with("Notice: this kit is valid."))
            .expect("notice heading");
        assert_eq!(
            lines[notice_at - 1],
            "",
            "notice is set apart by a blank line"
        );
        assert!(lines[notice_at].contains("'arkavo agent -c <kit>' does not enforce"));

        let notice = lines[notice_at..].join("\n");
        assert!(
            notice.contains(
                "  roles[].agent_provisioning.isolation (roles: critic)\n      \
                 sandbox, fs_writable and network_egress are not applied to the agent process"
            ),
            "{notice}"
        );
        assert!(
            notice.contains(
                "  constraints.network\n      \
                 egress_allowed and egress_allowlist do not restrict network access"
            ),
            "{notice}"
        );
        assert!(!notice.to_lowercase().contains("error"), "{notice}");
    }

    #[test]
    fn missing_file_is_reported_as_not_found() {
        let err = validate_kit_at(Path::new("no-such.swarmkit.yaml"), at(VALID_ON))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "kit file not found: no-such.swarmkit.yaml");
    }
}
