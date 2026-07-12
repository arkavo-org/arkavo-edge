//! Locate and load the primary SwarmKit config for a process.
//!
//! Product AGENTS.md is not read. If only AGENTS.md is present, callers get
//! [`DiscoverError::AgentsMdUnsupported`] with a migrate hint.

use std::path::{Path, PathBuf};

use crate::manifest::Manifest;
use crate::runtime_config::{AgentRuntimeConfig, agent_runtime_config_from_manifest};
use crate::{ParseError, parse_yaml};

/// Environment variable for an explicit kit path (gateway + CLI).
pub const SWARMKIT_PATH_ENV: &str = "ARKAVO_SWARMKIT_PATH";

/// Preferred config directory under the working tree.
pub const ARKAVO_DIR: &str = ".arkavo";

#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    #[error(
        "AGENTS.md is no longer supported as agent configuration. \
         Convert with: arkavo kit migrate-from-agents-md --in {path} --out agent.swarmkit.yaml"
    )]
    AgentsMdUnsupported { path: PathBuf },

    #[error("no SwarmKit manifest found (set {SWARMKIT_PATH_ENV} or add .arkavo/*.swarmkit.yaml)")]
    NotFound,

    #[error("multiple SwarmKit manifests in {dir}: {files:?}; set {SWARMKIT_PATH_ENV} to choose")]
    Multiple { dir: PathBuf, files: Vec<PathBuf> },

    #[error("read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("parse {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseError,
    },
}

/// Result of discovery: absolute path + validated manifest + process config view.
#[derive(Debug, Clone)]
pub struct DiscoveredKit {
    pub path: PathBuf,
    pub manifest: Manifest,
    pub config: AgentRuntimeConfig,
}

/// Discover kit path without parsing: env, then `.arkavo/`, then cwd.
pub fn discover_kit_path(cwd: &Path) -> Result<PathBuf, DiscoverError> {
    if let Ok(env_path) = std::env::var(SWARMKIT_PATH_ENV) {
        let p = PathBuf::from(env_path);
        if p.is_file() {
            return Ok(if p.is_absolute() { p } else { cwd.join(p) });
        }
        return Err(DiscoverError::Io {
            path: p,
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "ARKAVO_SWARMKIT_PATH not a file",
            ),
        });
    }

    let arkavo_dir = cwd.join(ARKAVO_DIR);
    if arkavo_dir.is_dir() {
        match list_swarmkit_files(&arkavo_dir)? {
            files if files.len() == 1 => return Ok(files.into_iter().next().unwrap()),
            files if files.len() > 1 => {
                return Err(DiscoverError::Multiple {
                    dir: arkavo_dir,
                    files,
                });
            }
            _ => {}
        }
    }

    match list_swarmkit_files(cwd)? {
        files if files.len() == 1 => return Ok(files.into_iter().next().unwrap()),
        files if files.len() > 1 => {
            return Err(DiscoverError::Multiple {
                dir: cwd.to_path_buf(),
                files,
            });
        }
        _ => {}
    }

    // Do not load AGENTS.md — fail with a clear migration message when present.
    for candidate in [
        cwd.join(ARKAVO_DIR).join("AGENTS.md"),
        cwd.join("AGENTS.md"),
    ] {
        if candidate.is_file() {
            return Err(DiscoverError::AgentsMdUnsupported { path: candidate });
        }
    }

    Err(DiscoverError::NotFound)
}

/// Load and validate the discovered kit, returning process-facing config.
pub fn load_discovered_kit(cwd: &Path) -> Result<DiscoveredKit, DiscoverError> {
    let path = discover_kit_path(cwd)?;
    load_kit_file(&path)
}

/// Load a specific kit file path.
pub fn load_kit_file(path: &Path) -> Result<DiscoveredKit, DiscoverError> {
    let content = std::fs::read_to_string(path).map_err(|source| DiscoverError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let manifest = parse_yaml(&content).map_err(|source| DiscoverError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    let config = agent_runtime_config_from_manifest(&manifest);
    Ok(DiscoveredKit {
        path: path.to_path_buf(),
        manifest,
        config,
    })
}

fn list_swarmkit_files(dir: &Path) -> Result<Vec<PathBuf>, DiscoverError> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|source| DiscoverError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| DiscoverError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_file() {
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name.ends_with(".swarmkit.yaml") || name.ends_with(".swarmkit.yml") {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_minimal_kit(dir: &Path, name: &str) {
        let yaml = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "hello"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  expires: "2026-05-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "say hello"
roles:
  - id: agent
    role_type: operator
    agent_provisioning: {}
    skills: []
    mcp_tools: []
    handoffs: []
coordination:
  topology: hub-spoke
  protocol: a2a-jsonrpc-2.0
  routing:
    strategy: static
constraints:
  global_budget:
    max_wallclock_seconds: 60
    max_total_tokens: 8000
    max_cost_usd: 0.01
  data_classifications: ["public"]
  network:
    egress_allowed: false
    egress_allowlist: []
completion:
  rules: ["done"]
  on_failure: abort
  max_retries: 0
provenance:
  signatures:
    - signer_did: "did:web:example.com"
      algorithm: ed25519
      signature: "AAA"
"#;
        fs::write(dir.join(name), yaml).unwrap();
    }

    #[arkavo_test_macros::spec("SK-102")]
    #[test]
    fn discovers_single_kit_in_arkavo_dir() {
        let dir = tempfile_dir();
        let arkavo = dir.join(".arkavo");
        fs::create_dir_all(&arkavo).unwrap();
        write_minimal_kit(&arkavo, "agent.swarmkit.yaml");
        let path = discover_kit_path(&dir).unwrap();
        assert!(path.ends_with("agent.swarmkit.yaml"));
        let discovered = load_discovered_kit(&dir).unwrap();
        assert_eq!(discovered.config.kit_name, "hello");
        assert_eq!(discovered.config.primary_role().unwrap().role_id, "agent");
        let _ = fs::remove_dir_all(&dir);
    }

    #[arkavo_test_macros::spec("SK-102")]
    #[test]
    fn agents_md_only_is_rejected() {
        let dir = tempfile_dir();
        fs::write(dir.join("AGENTS.md"), "# AGENTS.md\n## x\npurpose: y\n").unwrap();
        let err = discover_kit_path(&dir).unwrap_err();
        assert!(matches!(err, DiscoverError::AgentsMdUnsupported { .. }));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn multiple_kits_error() {
        let dir = tempfile_dir();
        write_minimal_kit(&dir, "a.swarmkit.yaml");
        write_minimal_kit(&dir, "b.swarmkit.yaml");
        let err = discover_kit_path(&dir).unwrap_err();
        assert!(matches!(err, DiscoverError::Multiple { .. }));
        let _ = fs::remove_dir_all(&dir);
    }

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "swarmkit-discover-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
