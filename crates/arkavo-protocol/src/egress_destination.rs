//! Where a call is about to send data (SEQ-003, SEQ-014).
//!
//! Destinations are read out of call parameters by *shape*, never by tool name.
//! A gate that knows the names of the tools that can exfiltrate is a gate that
//! any new tool walks straight past, and the set of names is exactly what an
//! attacker gets to choose.
//!
//! The extractor is deliberately eager: a string that could be a destination is
//! treated as one. A false positive costs an authorization check; a false
//! negative costs the disclosure the gate exists to prevent.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

/// Where data is headed, as far as the parameters reveal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Destination {
    /// A sanctioned endpoint inside the trust boundary.
    Internal { url: String },
    /// An endpoint outside it.
    External { url: String },
    /// A path inside the agent's workspace.
    Workspace { path: PathBuf },
    /// A path outside it — a write here leaves the sandbox.
    ExternalPath { path: PathBuf },
    /// A destination-shaped value that could not be resolved either way.
    ///
    /// Distinct from finding nothing: something is going somewhere and the gate
    /// cannot say where, which is a reason to hold rather than to allow.
    Unresolved { hint: String },
}

impl Destination {
    /// Whether releasing here amounts to disclosure outside the boundary.
    pub fn is_external(&self) -> bool {
        matches!(
            self,
            Destination::External { .. } | Destination::ExternalPath { .. }
        )
    }

    /// Coarse class, safe anywhere a refused caller might read.
    ///
    /// Separate from [`Destination::audit_detail`] because a denial that names
    /// the destination is a denial that confirms a guess about it.
    pub fn class(&self) -> &'static str {
        match self {
            Destination::Internal { .. } => "internal-endpoint",
            Destination::External { .. } => "external-url",
            Destination::Workspace { .. } => "workspace-path",
            Destination::ExternalPath { .. } => "external-path",
            Destination::Unresolved { .. } => "unresolved",
        }
    }

    /// Full identity of the destination. Audit sinks only.
    pub fn audit_detail(&self) -> String {
        match self {
            Destination::Internal { url } => format!("internal:{url}"),
            Destination::External { url } => format!("external:{url}"),
            Destination::Workspace { path } => format!("workspace:{}", path.display()),
            Destination::ExternalPath { path } => format!("path:{}", path.display()),
            Destination::Unresolved { hint } => format!("unresolved:{hint}"),
        }
    }
}

/// What counts as inside the boundary, and what can receive a wrapped payload.
#[derive(Debug, Clone, Default)]
pub struct DestinationPolicy {
    sanctioned_hosts: BTreeSet<String>,
    tdf_capable_hosts: BTreeSet<String>,
    workspace_root: Option<PathBuf>,
}

impl DestinationPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a host as inside the trust boundary (SEQ-003 edge case: a
    /// sanctioned internal endpoint).
    #[must_use]
    pub fn sanction_host(mut self, host: impl Into<String>) -> Self {
        self.sanctioned_hosts.insert(normalize_host(&host.into()));
        self
    }

    /// Mark a host as able to consume a TDF. A host not marked cannot, so a
    /// payload that needs wrapping to travel there cannot travel there at all.
    #[must_use]
    pub fn tdf_capable_host(mut self, host: impl Into<String>) -> Self {
        self.tdf_capable_hosts.insert(normalize_host(&host.into()));
        self
    }

    #[must_use]
    pub fn workspace_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.workspace_root = Some(root.into());
        self
    }

    pub fn is_sanctioned(&self, host: &str) -> bool {
        self.sanctioned_hosts.contains(&normalize_host(host))
    }

    /// Whether a destination can receive a wrapped payload.
    ///
    /// A remote peer must be declared: assuming one understands a TDF and being
    /// wrong means shipping a blob it stores or forwards as opaque bytes.
    pub fn can_consume_tdf(&self, destination: &Destination) -> bool {
        match destination {
            // A path inside the workspace stays under the agent's own root, so
            // a wrapped file there is still governed. A path outside it is not
            // a consumer of anything — whoever picks the file up has no key
            // request path, so "wrapped" there is just bytes leaving.
            Destination::Workspace { .. } => true,
            Destination::ExternalPath { .. } => false,
            Destination::Internal { url } | Destination::External { url } => {
                host_of(url).is_some_and(|h| self.tdf_capable_hosts.contains(&normalize_host(&h)))
            }
            Destination::Unresolved { .. } => false,
        }
    }

    fn classify_url(&self, url: &str) -> Destination {
        match host_of(url) {
            Some(host) if self.is_sanctioned(&host) => Destination::Internal {
                url: url.to_string(),
            },
            Some(_) => Destination::External {
                url: url.to_string(),
            },
            None => Destination::Unresolved {
                hint: "url".to_string(),
            },
        }
    }

    fn classify_path(&self, raw: &str) -> Destination {
        let given = PathBuf::from(raw);
        match &self.workspace_root {
            // With no workspace declared, no path can be shown to stay inside
            // one. Held rather than allowed: the gate does not get to assume.
            None => Destination::Unresolved {
                hint: "path".to_string(),
            },
            Some(root) => {
                // A relative path resolves against the workspace, so it has to
                // be joined before the prefix test — otherwise `notes.md` fails
                // to start with the root and reads as an escape, and
                // `../../etc/passwd` reads as one only by accident.
                let resolved = if given.is_absolute() {
                    normalize(&given)
                } else {
                    normalize(&root.join(&given))
                };
                if resolved.starts_with(normalize(root)) {
                    Destination::Workspace { path: resolved }
                } else {
                    Destination::ExternalPath { path: resolved }
                }
            }
        }
    }
}

/// Every destination the parameters name, deduplicated and ordered.
pub fn extract_destinations(params: &Value, policy: &DestinationPolicy) -> Vec<Destination> {
    let mut found = BTreeSet::new();
    walk(params, policy, &mut found);
    found.into_iter().collect()
}

fn walk(value: &Value, policy: &DestinationPolicy, found: &mut BTreeSet<Destination>) {
    match value {
        Value::String(s) => {
            if let Some(destination) = classify(s, policy) {
                found.insert(destination);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(item, policy, found);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                walk(item, policy, found);
            }
        }
        _ => {}
    }
}

fn classify(s: &str, policy: &DestinationPolicy) -> Option<Destination> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(scheme_end) = trimmed.find("://") {
        let scheme = trimmed[..scheme_end].to_ascii_lowercase();
        return Some(match scheme.as_str() {
            "http" | "https" => policy.classify_url(trimmed),
            "file" => policy.classify_path(trimmed.trim_start_matches("file://")),
            // Any other scheme still moves data somewhere the gate cannot
            // reason about — ftp, s3, a custom peer transport.
            _ => Destination::Unresolved { hint: scheme },
        });
    }

    if looks_like_path(trimmed) || looks_like_relative_file(trimmed) {
        return Some(policy.classify_path(trimmed));
    }

    None
}

/// Absolute paths and traversal-bearing relative paths.
fn looks_like_path(s: &str) -> bool {
    if s.contains(char::is_whitespace) {
        return false;
    }
    s.starts_with('/')
        || s.starts_with("~/")
        || s.starts_with("../")
        || s.contains("/../")
        || (s.len() > 2 && s.as_bytes()[1] == b':' && (s.contains('\\') || s.contains('/')))
}

/// A bare relative name that still reads as a file: `notes.md`, `out/data.csv`.
///
/// These have to be classified rather than skipped. A tool that writes to a
/// caller-supplied relative path is exactly the shape this gate exists to
/// catch, and skipping them left `extract_destinations` reporting nothing, so
/// the call was never evaluated at all. Resolving them against the workspace
/// root is what decides whether they stay inside it.
///
/// Deliberately narrow: a token only qualifies with a path separator or a
/// short file extension. Widening it to every unspaced string would classify
/// ordinary prose and identifiers as destinations, and with no workspace root
/// configured that resolves to `Unresolved`, which holds. A gate that holds the
/// session on its own output is a gate that gets turned off.
fn looks_like_relative_file(s: &str) -> bool {
    if s.contains(char::is_whitespace) || s.contains("://") {
        return false;
    }
    if s.contains('/') {
        return true;
    }
    s.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && !ext.is_empty()
            && ext.len() <= 8
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
    })
}

/// Resolve `.` and `..` lexically. The gate runs before the write, so the path
/// need not exist, which rules out canonicalizing against the filesystem.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Host extraction is the validation crate's, deliberately: the gate must see
/// exactly the host that the SSRF filter and the transport will see, or the two
/// disagree about what was contacted.
fn host_of(url: &str) -> Option<String> {
    arkavo_validation::extract_host_from_url(url)
}

fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn policy() -> DestinationPolicy {
        DestinationPolicy::new()
            .sanction_host("vault.internal")
            .workspace_root("/work/agent")
    }

    #[test]
    fn an_external_url_anywhere_in_the_params_is_found() {
        let params = json!({"body": {"nested": ["https://attacker.example/collect"]}});

        let found = extract_destinations(&params, &policy());

        assert_eq!(
            found,
            vec![Destination::External {
                url: "https://attacker.example/collect".into()
            }]
        );
    }

    #[test]
    fn a_sanctioned_host_is_internal() {
        let params = json!({"url": "https://vault.internal/secrets"});

        let found = extract_destinations(&params, &policy());

        assert!(matches!(found[0], Destination::Internal { .. }));
    }

    #[test]
    fn host_matching_ignores_case_and_trailing_dot() {
        let params = json!({"url": "https://VAULT.Internal./secrets"});

        let found = extract_destinations(&params, &policy());

        assert!(matches!(found[0], Destination::Internal { .. }));
    }

    #[test]
    fn a_path_inside_the_workspace_is_a_workspace_write() {
        let params = json!({"path": "/work/agent/notes.md"});

        let found = extract_destinations(&params, &policy());

        assert!(matches!(found[0], Destination::Workspace { .. }));
    }

    #[test]
    fn a_path_outside_the_workspace_is_external() {
        let params = json!({"path": "/etc/cron.d/exfil"});

        let found = extract_destinations(&params, &policy());

        assert!(matches!(found[0], Destination::ExternalPath { .. }));
    }

    #[test]
    fn traversal_out_of_the_workspace_is_external() {
        // Lexical normalization has to happen before the prefix test, or
        // `/work/agent/../../etc/x` reads as a workspace path.
        let params = json!({"path": "/work/agent/../../etc/x"});

        let found = extract_destinations(&params, &policy());

        assert!(
            matches!(found[0], Destination::ExternalPath { .. }),
            "traversal escaped the workspace check: {found:?}"
        );
    }

    #[test]
    fn an_unknown_scheme_is_unresolved_not_ignored() {
        let params = json!({"sink": "s3://bucket/key"});

        let found = extract_destinations(&params, &policy());

        assert_eq!(found, vec![Destination::Unresolved { hint: "s3".into() }]);
    }

    #[test]
    fn a_path_with_no_workspace_declared_is_unresolved() {
        let bare = DestinationPolicy::new();
        let params = json!({"path": "/tmp/out"});

        let found = extract_destinations(&params, &bare);

        assert_eq!(
            found,
            vec![Destination::Unresolved {
                hint: "path".into()
            }]
        );
    }

    #[test]
    fn prose_is_not_a_destination() {
        let params = json!({"prompt": "summarize the report and/or the appendix"});

        assert!(extract_destinations(&params, &policy()).is_empty());
    }

    #[test]
    fn a_bare_relative_filename_is_classified_not_skipped() {
        // Regression: these returned no destination at all, so the call fell
        // through the gate's empty-destinations early return unevaluated.
        let params = json!({"path": "notes.md"});

        let found = extract_destinations(&params, &policy());

        assert_eq!(
            found,
            vec![Destination::Workspace {
                path: "/work/agent/notes.md".into()
            }]
        );
    }

    #[test]
    fn a_relative_subdirectory_write_is_classified() {
        let params = json!({"path": "exfil/data.txt"});

        let found = extract_destinations(&params, &policy());

        assert!(
            matches!(found[0], Destination::Workspace { .. }),
            "{found:?}"
        );
    }

    #[test]
    fn a_relative_path_escaping_the_workspace_is_external() {
        let params = json!({"path": "../../etc/passwd"});

        let found = extract_destinations(&params, &policy());

        assert!(
            matches!(found[0], Destination::ExternalPath { .. }),
            "{found:?}"
        );
    }

    #[test]
    fn a_bare_identifier_is_not_a_destination() {
        // Without this, a model name or a tool argument would resolve to
        // Unresolved wherever no workspace root is configured, holding the
        // session on its own ordinary output.
        let params = json!({"model": "qwen3", "mode": "fast", "n": "12"});

        assert!(extract_destinations(&params, &policy()).is_empty());
    }

    #[test]
    fn extraction_does_not_depend_on_the_parameter_name() {
        // The same URL under a name the gate has never seen must still be found.
        let params = json!({"totally_unknown_field": "https://attacker.example/x"});

        assert_eq!(extract_destinations(&params, &policy()).len(), 1);
    }

    #[test]
    fn workspace_paths_can_receive_a_wrapped_payload_but_unknown_hosts_cannot() {
        let policy = policy().tdf_capable_host("peer.example");

        assert!(policy.can_consume_tdf(&Destination::Workspace {
            path: "/work/agent/x".into()
        }));
        assert!(policy.can_consume_tdf(&Destination::External {
            url: "https://peer.example/inbox".into()
        }));
        assert!(!policy.can_consume_tdf(&Destination::External {
            url: "https://attacker.example/x".into()
        }));
        assert!(!policy.can_consume_tdf(&Destination::Unresolved { hint: "s3".into() }));
    }
}
