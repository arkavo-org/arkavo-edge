//! Read the skills out of a kit file that is still being authored.
//!
//! Signing tools need the skill text exactly as it stands in the file, at a
//! point where the kit cannot pass validation yet: the author has just
//! edited a skill, so a declared `kit.id` no longer matches. This reads only
//! the skills and skips cross-block validation. It returns no manifest, so
//! it cannot be used to load an unvalidated kit for running.

use crate::manifest::Manifest;
use crate::role::SkillSource;
use crate::skill_content::SkillContent;

/// One skill reference from a kit file, with its content when the kit
/// carries it inline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KitSkill {
    pub role_id: String,
    pub id: String,
    pub version: String,
    pub source: SkillSource,
    /// `None` when the kit does not carry the content: the skill has no
    /// payload, or its source is a registry or TDF reference.
    pub content: Option<SkillContent>,
}

#[derive(Debug, thiserror::Error)]
pub enum KitSkillsError {
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("role {role:?}, skill {skill:?}: payload is not skill content: {source}")]
    Payload {
        role: String,
        skill: String,
        #[source]
        source: serde_json::Error,
    },
}

/// Every skill in `yaml`, in manifest order.
pub fn kit_skills_from_yaml(yaml: &str) -> Result<Vec<KitSkill>, KitSkillsError> {
    let manifest: Manifest = serde_yaml::from_str(yaml)?;
    let mut skills = Vec::new();
    for role in &manifest.roles {
        for skill in &role.skills {
            let content = match (&skill.source, &skill.payload) {
                (SkillSource::Inline, Some(payload)) => Some(
                    serde_json::from_value::<SkillContent>(payload.clone()).map_err(|source| {
                        KitSkillsError::Payload {
                            role: role.id.clone(),
                            skill: skill.id.clone(),
                            source,
                        }
                    })?,
                ),
                _ => None,
            };
            skills.push(KitSkill {
                role_id: role.id.clone(),
                id: skill.id.clone(),
                version: skill.version.clone(),
                source: skill.source,
                content,
            });
        }
    }
    Ok(skills)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kit(id: &str, skills: &str) -> String {
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
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "review"
roles:
  - id: planner
    role_type: planner
    agent_provisioning: {{}}
    skills:
{skills}
  - id: critic
    role_type: critic
    agent_provisioning: {{}}
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

    const INLINE: &str = r#"      - id: "skill:plan"
        version: "1.2.0"
        source: inline
        payload:
          name: plan
          description: "Plan the work."
          instructions: "Write the plan as written in the file."
          resources: []"#;

    #[test]
    fn returns_inline_skill_text_as_written_in_the_file() {
        let skills = kit_skills_from_yaml(&kit("", INLINE)).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].role_id, "planner");
        assert_eq!(skills[0].id, "skill:plan");
        assert_eq!(skills[0].version, "1.2.0");
        assert_eq!(
            skills[0].content.as_ref().unwrap().instructions,
            "Write the plan as written in the file."
        );
    }

    /// A kit whose skill was just edited still declares the id of the old
    /// content. Signing comes before the id is recomputed, so reading the
    /// skills must not depend on the id being current.
    #[test]
    fn reads_skills_from_a_kit_whose_id_is_stale() {
        let yaml = kit("blake3:id-of-the-text-before-the-edit", INLINE);
        assert!(
            crate::parse_yaml(&yaml).is_err(),
            "fixture must be a kit that does not validate"
        );
        let skills = kit_skills_from_yaml(&yaml).unwrap();
        assert_eq!(skills[0].content.as_ref().unwrap().name, "plan");
    }

    #[test]
    fn skills_without_inline_content_are_listed_with_no_content() {
        let skills = r#"      - id: "skill:remote"
        version: "1.0.0"
        source: registry
      - id: "skill:empty"
        version: "1.0.0"
        source: inline"#;
        let skills = kit_skills_from_yaml(&kit("", skills)).unwrap();
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].source, SkillSource::Registry);
        assert!(skills.iter().all(|s| s.content.is_none()));
    }

    #[test]
    fn malformed_payload_names_the_role_and_skill() {
        let skills = r#"      - id: "skill:broken"
        version: "1.0.0"
        source: inline
        payload:
          name: broken"#;
        let err = kit_skills_from_yaml(&kit("", skills))
            .unwrap_err()
            .to_string();
        assert!(err.contains("role \"planner\""), "{err}");
        assert!(err.contains("skill \"skill:broken\""), "{err}");
    }
}
