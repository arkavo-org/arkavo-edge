#![allow(clippy::collapsible_if)]

use std::collections::HashMap;
use std::path::PathBuf;

pub(crate) fn get_model_compatibility(model_name: &str) -> (&'static str, &'static str) {
    let name_lower = model_name.to_lowercase();
    if name_lower.contains("qwen") {
        ("compatible", "Qwen3")
    } else if name_lower.contains("mistral") || name_lower.contains("ministral") {
        ("compatible", "MistralV3")
    } else if name_lower.contains("gemma") {
        ("compatible", "Gemma3")
    } else if name_lower.contains("glm-4") || name_lower.contains("glm4") {
        ("compatible", "GLM4")
    } else {
        ("incompatible", "unknown format")
    }
}

pub(crate) fn parse_agents_config() -> HashMap<String, Vec<String>> {
    let mut models = HashMap::new();

    let mut current_dir = std::env::current_dir().ok();
    let mut agents_file = None;

    while let Some(dir) = current_dir {
        let candidate = dir.join(".arkavo").join("AGENTS.md");
        if candidate.exists() {
            agents_file = Some(candidate);
            break;
        }
        current_dir = dir.parent().map(|p| p.to_path_buf());
    }

    if let Some(path) = agents_file {
        if let Ok(content) = std::fs::read_to_string(path) {
            let mut current_provider = String::new();

            for line in content.lines() {
                let trimmed = line.trim();

                if trimmed.starts_with("##") {
                    current_provider = trimmed.trim_start_matches("##").trim().to_string();
                    models
                        .entry(current_provider.clone())
                        .or_insert_with(Vec::new);
                } else if trimmed.contains("API_KEY=") {
                    if let Some((key, value)) = trimmed.split_once('=') {
                        let key = key.trim();
                        let value = value.trim();
                        if std::env::var(key).is_err() {
                            // SAFETY: set during config parse, before worker threads.
                            unsafe {
                                std::env::set_var(key, value);
                            }
                        }
                    }
                } else if trimmed.starts_with('-') && !current_provider.is_empty() {
                    let model = trimmed
                        .trim_start_matches('-')
                        .trim()
                        .split(':')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string();

                    if !model.is_empty() {
                        models
                            .entry(current_provider.clone())
                            .or_insert_with(Vec::new)
                            .push(model);
                    }
                }
            }
        }
    }

    models
}

pub(crate) fn list_local_gguf_models() -> Vec<(String, String, PathBuf, u64)> {
    let hf_cache_dir = if let Ok(hf_home) = std::env::var("HF_HOME") {
        Some(PathBuf::from(hf_home).join("hub"))
    } else {
        dirs::home_dir().map(|d| d.join(".cache").join("huggingface").join("hub"))
    };

    match hf_cache_dir {
        Some(cache_dir) => list_gguf_models_in(&cache_dir),
        None => Vec::new(),
    }
}

/// Scan a Hugging Face hub directory (the `hub` dir under `HF_HOME`, e.g.
/// `~/.cache/huggingface/hub`) for `.gguf` and `.gguf.tdf` model files.
///
/// Pure function of the given path so it can be exercised against a
/// `tempfile::tempdir()` fixture instead of the real, process-wide HF cache.
fn list_gguf_models_in(hub: &std::path::Path) -> Vec<(String, String, PathBuf, u64)> {
    let mut found_models = Vec::new();

    if !hub.exists() {
        return found_models;
    }

    let Ok(entries) = std::fs::read_dir(hub) else {
        return found_models;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !dir_name.starts_with("models--") {
            continue;
        }

        let snapshots_dir = path.join("snapshots");
        if !snapshots_dir.exists() {
            continue;
        }
        let Ok(snapshot_entries) = std::fs::read_dir(&snapshots_dir) else {
            continue;
        };

        for snapshot in snapshot_entries.flatten() {
            let snapshot_path = snapshot.path();
            if !snapshot_path.is_dir() {
                continue;
            }
            let Ok(files) = std::fs::read_dir(&snapshot_path) else {
                continue;
            };

            for file in files.flatten() {
                let Some(name) = file.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                if !is_listed_gguf(&name) {
                    continue;
                }
                let model_name = dir_name
                    .strip_prefix("models--")
                    .unwrap_or(dir_name)
                    .replace("--", "/");
                let file_path = file.path();
                let size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
                found_models.push((model_name, name, file_path, size));
            }
        }
    }

    found_models
}

fn is_listed_gguf(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".gguf.tdf") || lower.ends_with(".gguf")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_both_plaintext_and_protected_gguf_names() {
        assert!(is_listed_gguf("model.gguf"));
        assert!(is_listed_gguf("model.GGUF"));
        assert!(is_listed_gguf("model.gguf.tdf"));
        assert!(is_listed_gguf("Model.GGUF.TDF"));
        assert!(!is_listed_gguf("notes.txt"));
        assert!(!is_listed_gguf("model.tdf"));
    }

    /// A HF hub snapshot that carries both `model.gguf` and its rewrapped
    /// `model.gguf.tdf` sibling must list both artifacts, not just one.
    #[test]
    fn lists_both_artifacts_when_plaintext_and_protected_are_side_by_side() {
        let hub = tempfile::tempdir().unwrap();
        let snapshot_dir = hub
            .path()
            .join("models--org--demo")
            .join("snapshots")
            .join("abc123");
        std::fs::create_dir_all(&snapshot_dir).unwrap();
        std::fs::write(snapshot_dir.join("model.gguf"), b"GGUF-plain").unwrap();
        std::fs::write(snapshot_dir.join("model.gguf.tdf"), b"GGUF-protected").unwrap();
        // A non-model file in the same snapshot must not be picked up.
        std::fs::write(snapshot_dir.join("README.md"), b"not a model").unwrap();

        let found = list_gguf_models_in(hub.path());

        let names: Vec<&str> = found.iter().map(|(_, name, _, _)| name.as_str()).collect();
        assert!(
            names.contains(&"model.gguf"),
            "expected model.gguf in {names:?}"
        );
        assert!(
            names.contains(&"model.gguf.tdf"),
            "expected model.gguf.tdf in {names:?}"
        );
        assert_eq!(found.len(), 2, "expected exactly the two gguf artifacts");

        for (model_name, _, path, size) in &found {
            assert_eq!(model_name, "org/demo");
            assert!(path.starts_with(&snapshot_dir));
            assert!(*size > 0);
        }
    }

    #[test]
    fn returns_empty_when_hub_directory_does_not_exist() {
        let hub = tempfile::tempdir().unwrap();
        let missing = hub.path().join("does-not-exist");
        assert!(list_gguf_models_in(&missing).is_empty());
    }
}
