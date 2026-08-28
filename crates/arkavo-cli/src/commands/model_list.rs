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
    let mut found_models = Vec::new();

    let hf_cache_dir = if let Ok(hf_home) = std::env::var("HF_HOME") {
        Some(PathBuf::from(hf_home).join("hub"))
    } else {
        dirs::home_dir().map(|d| d.join(".cache").join("huggingface").join("hub"))
    };

    if let Some(cache_dir) = hf_cache_dir {
        if cache_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&cache_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                        if dir_name.starts_with("models--") {
                            let snapshots_dir = path.join("snapshots");
                            if snapshots_dir.exists() {
                                if let Ok(snapshot_entries) = std::fs::read_dir(&snapshots_dir) {
                                    for snapshot in snapshot_entries.flatten() {
                                        let snapshot_path = snapshot.path();
                                        if snapshot_path.is_dir() {
                                            if let Ok(files) = std::fs::read_dir(&snapshot_path) {
                                                for file in files.flatten() {
                                                    if let Some(name) = file.file_name().to_str() {
                                                        if is_listed_gguf(name) {
                                                            let model_name = dir_name
                                                                .strip_prefix("models--")
                                                                .unwrap_or(dir_name)
                                                                .replace("--", "/");
                                                            let file_path = file.path();
                                                            let size =
                                                                std::fs::metadata(&file_path)
                                                                    .map(|m| m.len())
                                                                    .unwrap_or(0);
                                                            found_models.push((
                                                                model_name,
                                                                name.to_string(),
                                                                file_path,
                                                                size,
                                                            ));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
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
}
