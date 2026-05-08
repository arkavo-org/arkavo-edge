//! SkillContent — the *contents* of a skill per spec §8.1 ("the SKILL.md
//! pattern: name, description, instructions, optional bundled resources").
//!
//! Distinct from `Skill`, which is the *reference* to a skill (id + version
//! + source).
//!
//! The resolver in arkavo-swarmkit-runtime parses a skill's payload (or a
//! registry-cached file) into this struct.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillContent {
    pub name: String,
    pub description: String,
    pub instructions: String,
    #[serde(default)]
    pub resources: Vec<SkillResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillResource {
    pub name: String,
    pub mime: String,
    /// Base64url (no padding) encoded bytes.
    pub bytes_base64: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let content = SkillContent {
            name: "asset-analysis".into(),
            description: "Summarize a source asset".into(),
            instructions: "Extract three to five selling points.".into(),
            resources: vec![],
        };
        let json = serde_json::to_string(&content).unwrap();
        let back: SkillContent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, content);
    }

    #[test]
    fn parses_with_default_resources() {
        let json = r#"{"name":"x","description":"y","instructions":"z"}"#;
        let parsed: SkillContent = serde_json::from_str(json).unwrap();
        assert!(parsed.resources.is_empty());
    }
}
