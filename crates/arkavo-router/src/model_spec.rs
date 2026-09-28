use crate::decision::ModelChoice;
use std::path::{Path, PathBuf};

/// CLI / session override: a catalog model or an on-disk GGUF path.
///
/// Path overrides are not registered as named [`ModelChoice`] entries.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelSpec {
    Named(ModelChoice),
    GgufPath(PathBuf),
}

impl ModelSpec {
    /// Resolve `--model` / `--gguf`: catalog names first, then a `.gguf` path.
    pub fn parse(spec: &str) -> Option<Self> {
        if let Some(model) = ModelChoice::from_name(spec) {
            return Some(Self::Named(model));
        }
        if is_gguf_spec(spec) {
            return Some(Self::GgufPath(PathBuf::from(spec)));
        }
        None
    }

    pub fn display_name(&self) -> String {
        match self {
            Self::Named(model) => model.name().to_string(),
            Self::GgufPath(path) => path.display().to_string(),
        }
    }

    pub fn as_named(&self) -> Option<&ModelChoice> {
        match self {
            Self::Named(model) => Some(model),
            Self::GgufPath(_) => None,
        }
    }

    pub fn as_gguf_path(&self) -> Option<&Path> {
        match self {
            Self::Named(_) => None,
            Self::GgufPath(path) => Some(path),
        }
    }
}

/// True when `spec` names a plaintext GGUF or a protected `.gguf.tdf`.
pub fn is_gguf_spec(spec: &str) -> bool {
    let lower = spec.to_ascii_lowercase();
    lower.ends_with(".gguf") || lower.ends_with(".gguf.tdf")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_prefers_catalog_name() {
        assert_eq!(
            ModelSpec::parse("qwen3.5-0.8b"),
            Some(ModelSpec::Named(ModelChoice::LocalQwen3))
        );
        assert_eq!(
            ModelSpec::parse("ministral-3b"),
            Some(ModelSpec::Named(ModelChoice::LocalMinistral3B))
        );
    }

    #[test]
    fn parse_accepts_gguf_and_tdf_paths() {
        let gguf = ModelSpec::parse("models/org/adapter.gguf");
        assert_eq!(
            gguf,
            Some(ModelSpec::GgufPath(PathBuf::from(
                "models/org/adapter.gguf"
            )))
        );
        assert!(gguf.as_ref().and_then(ModelSpec::as_named).is_none());
        assert_eq!(
            gguf.as_ref()
                .and_then(ModelSpec::as_gguf_path)
                .map(Path::to_str),
            Some(Some("models/org/adapter.gguf"))
        );

        let tdf = ModelSpec::parse("/tmp/model.gguf.tdf");
        assert_eq!(
            tdf,
            Some(ModelSpec::GgufPath(PathBuf::from("/tmp/model.gguf.tdf")))
        );
    }

    #[test]
    fn parse_gguf_is_case_insensitive() {
        assert!(matches!(
            ModelSpec::parse("Adapter.GGUF"),
            Some(ModelSpec::GgufPath(_))
        ));
        assert!(matches!(
            ModelSpec::parse("Adapter.GGUF.TDF"),
            Some(ModelSpec::GgufPath(_))
        ));
    }

    #[test]
    fn parse_rejects_unknown_names_and_non_gguf_paths() {
        assert_eq!(ModelSpec::parse("unknown-model"), None);
        assert_eq!(ModelSpec::parse("models/org/adapter.bin"), None);
        assert_eq!(ModelSpec::parse("oida-mallinckrodt-knowledge"), None);
    }

    #[test]
    fn is_gguf_spec_does_not_treat_catalog_names_as_paths() {
        assert!(!is_gguf_spec("qwen3.5-0.8b"));
        assert!(!is_gguf_spec("ministral-3b"));
        assert!(is_gguf_spec("./oida-mallinckrodt-knowledge.gguf"));
    }
}
