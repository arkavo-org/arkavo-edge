//! Filename-based dispatch helpers shared by every entry point that
//! loads a SwarmKit manifest from disk (gateway auto-launch,
//! orchestrator apply-kit, CLI tooling).
//!
//! Both the AG-UI gateway and the orchestrator need to know whether a
//! path is a plaintext YAML/JSON manifest or a `.swarmkit.tdf`
//! envelope. Centralizing the rule here keeps the two consumers in
//! agreement and avoids duplicating the recognized-extension table.

use std::path::Path;

/// Classification of a SwarmKit manifest path by file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KitPathKind {
    /// `*.yaml` / `*.yml` / `*.swarmkit.yaml`.
    Yaml,
    /// `*.json`.
    Json,
    /// `*.tdf` / `*.swarmkit.tdf` — encrypted envelope.
    Tdf,
    /// Anything else, including no extension.
    Unknown,
}

impl KitPathKind {
    pub fn is_tdf(self) -> bool {
        matches!(self, KitPathKind::Tdf)
    }
}

/// Classify a path by its file extension. The recognized table:
/// * `*.tdf`, `*.swarmkit.tdf` → [`KitPathKind::Tdf`]
/// * `*.yaml`, `*.yml`, `*.swarmkit.yaml` → [`KitPathKind::Yaml`]
/// * `*.json` → [`KitPathKind::Json`]
/// * everything else → [`KitPathKind::Unknown`]
pub fn classify_kit_path(path: &Path) -> KitPathKind {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return KitPathKind::Unknown;
    };
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".tdf") || lower.ends_with(".swarmkit.tdf") {
        KitPathKind::Tdf
    } else if lower.ends_with(".yaml")
        || lower.ends_with(".yml")
        || lower.ends_with(".swarmkit.yaml")
    {
        KitPathKind::Yaml
    } else if lower.ends_with(".json") {
        KitPathKind::Json
    } else {
        KitPathKind::Unknown
    }
}

/// Convenience predicate. `true` when the path's extension marks it as
/// a TDF envelope (bare `.tdf` or `.swarmkit.tdf`).
pub fn is_tdf_path(path: &Path) -> bool {
    classify_kit_path(path).is_tdf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn classifies_tdf_extensions() {
        assert!(is_tdf_path(&PathBuf::from("kit.tdf")));
        assert!(is_tdf_path(&PathBuf::from("kit.swarmkit.tdf")));
        assert!(is_tdf_path(&PathBuf::from("/abs/path/Kit.SWARMKIT.TDF")));
    }

    #[test]
    fn classifies_yaml_and_json_as_non_tdf() {
        assert!(!is_tdf_path(&PathBuf::from("kit.swarmkit.yaml")));
        assert!(!is_tdf_path(&PathBuf::from("kit.yaml")));
        assert!(!is_tdf_path(&PathBuf::from("kit.yml")));
        assert!(!is_tdf_path(&PathBuf::from("kit.json")));
    }

    #[test]
    fn classifies_unknown_extensions() {
        assert!(!is_tdf_path(&PathBuf::from("kit")));
        assert_eq!(
            classify_kit_path(&PathBuf::from("notes.md")),
            KitPathKind::Unknown
        );
    }

    #[test]
    fn classify_returns_full_kind() {
        assert_eq!(
            classify_kit_path(&PathBuf::from("kit.swarmkit.yaml")),
            KitPathKind::Yaml
        );
        assert_eq!(
            classify_kit_path(&PathBuf::from("kit.json")),
            KitPathKind::Json
        );
        assert_eq!(
            classify_kit_path(&PathBuf::from("kit.swarmkit.tdf")),
            KitPathKind::Tdf
        );
    }
}
