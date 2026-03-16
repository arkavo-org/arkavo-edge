use crate::ast_op::AstOp;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// The atomic unit of code change in EvoFabric.
///
/// Every agent emission is an `OpBundle`, not a patch. Each bundle targets
/// exactly one file and contains one or more typed AST operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpBundle {
    pub id: Uuid,
    pub originator: String,
    pub target_file: String,
    pub ops: Vec<AstOp>,
    pub rationale: String,
    pub created_at: DateTime<Utc>,
    #[serde(
        serialize_with = "serialize_hash",
        deserialize_with = "deserialize_hash"
    )]
    pub content_hash: [u8; 32],
    pub status: OpBundleStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state")]
pub enum OpBundleStatus {
    Proposed,
    Verified,
    Applied,
    Rejected { reason: String },
}

impl OpBundle {
    /// Create a new proposed `OpBundle`.
    pub fn new(
        originator: String,
        target_file: String,
        ops: Vec<AstOp>,
        rationale: String,
    ) -> Self {
        let mut bundle = Self {
            id: Uuid::new_v4(),
            originator,
            target_file,
            ops,
            rationale,
            created_at: Utc::now(),
            content_hash: [0; 32],
            status: OpBundleStatus::Proposed,
        };
        bundle.content_hash = bundle.compute_hash();
        bundle
    }

    /// Compute the SHA-256 hash of the serialized ops.
    pub fn compute_hash(&self) -> [u8; 32] {
        let ops_json = serde_json::to_string(&self.ops).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(self.target_file.as_bytes());
        hasher.update(ops_json.as_bytes());
        hasher.finalize().into()
    }

    /// Parse an `OpBundle` from LLM JSON output.
    ///
    /// Performs lightweight cleanup before parsing:
    /// 1. Strips markdown fences (```json ... ```)
    /// 2. Strips trailing commas before `}` or `]`
    pub fn from_json(raw: &str) -> crate::Result<Self> {
        let cleaned = cleanup_llm_json(raw);
        let mut bundle: Self = serde_json::from_str(&cleaned)?;
        bundle.content_hash = bundle.compute_hash();
        if bundle.id.is_nil() {
            bundle.id = Uuid::new_v4();
        }
        Ok(bundle)
    }
}

/// Clean up common LLM JSON output issues.
fn cleanup_llm_json(raw: &str) -> String {
    let mut s = raw.trim().to_string();

    // Strip markdown fences
    if let Some(start) = s.find("```") {
        let after_fence = &s[start + 3..];
        // Skip optional language tag (e.g. "json")
        let content_start = after_fence.find('\n').map_or(0, |i| i + 1);
        let content = &after_fence[content_start..];
        if let Some(end) = content.rfind("```") {
            s = content[..end].to_string();
        }
    }

    // Strip trailing commas before } or ]
    let trailing_comma = regex::Regex::new(r",\s*([}\]])").unwrap();
    trailing_comma.replace_all(&s, "$1").into_owned()
}

fn serialize_hash<S: serde::Serializer>(hash: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&hex::encode(hash))
}

fn deserialize_hash<'de, D: serde::Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
    let s = String::deserialize(d)?;
    // Allow empty/zero hash (will be recomputed)
    if s.is_empty() || s.chars().all(|c| c == '0') {
        return Ok([0; 32]);
    }
    let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
    bytes
        .try_into()
        .map_err(|_| serde::de::Error::custom("expected 32-byte hash"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target_scope::TargetScope;

    #[test]
    fn new_bundle_computes_hash() {
        let bundle = OpBundle::new(
            "agent-1".into(),
            "src/lib.rs".into(),
            vec![AstOp::AddUse {
                path: "std::io".into(),
            }],
            "add io import".into(),
        );
        assert_ne!(bundle.content_hash, [0; 32]);
        assert_eq!(bundle.status, OpBundleStatus::Proposed);
    }

    #[test]
    fn hash_is_deterministic() {
        let ops = vec![AstOp::Remove {
            scope: TargetScope::Function {
                name: "foo".into(),
                within: None,
            },
        }];
        let b1 = OpBundle::new("a".into(), "f.rs".into(), ops.clone(), "r".into());
        let b2 = OpBundle::new("a".into(), "f.rs".into(), ops, "r".into());
        assert_eq!(b1.content_hash, b2.content_hash);
    }

    #[test]
    fn from_json_valid() {
        let json = r#"{
            "id": "00000000-0000-0000-0000-000000000000",
            "originator": "test",
            "target_file": "src/lib.rs",
            "ops": [{"op": "AddUse", "path": "std::io"}],
            "rationale": "need io",
            "created_at": "2026-01-01T00:00:00Z",
            "content_hash": "",
            "status": {"state": "Proposed"}
        }"#;
        let bundle = OpBundle::from_json(json).unwrap();
        assert_eq!(bundle.target_file, "src/lib.rs");
        assert_ne!(bundle.content_hash, [0; 32]);
    }

    #[test]
    fn from_json_with_markdown_fences() {
        let json = r#"```json
{
    "id": "00000000-0000-0000-0000-000000000000",
    "originator": "test",
    "target_file": "src/lib.rs",
    "ops": [{"op": "AddUse", "path": "std::io"}],
    "rationale": "need io",
    "created_at": "2026-01-01T00:00:00Z",
    "content_hash": "",
    "status": {"state": "Proposed"}
}
```"#;
        let bundle = OpBundle::from_json(json).unwrap();
        assert_eq!(bundle.target_file, "src/lib.rs");
    }

    #[test]
    fn from_json_with_trailing_commas() {
        let json = r#"{
            "id": "00000000-0000-0000-0000-000000000000",
            "originator": "test",
            "target_file": "src/lib.rs",
            "ops": [{"op": "AddUse", "path": "std::io",},],
            "rationale": "need io",
            "created_at": "2026-01-01T00:00:00Z",
            "content_hash": "",
            "status": {"state": "Proposed",},
        }"#;
        let bundle = OpBundle::from_json(json).unwrap();
        assert_eq!(bundle.target_file, "src/lib.rs");
    }

    #[test]
    fn from_json_invalid_returns_error() {
        assert!(OpBundle::from_json("not json at all").is_err());
    }

    #[test]
    fn serde_roundtrip() {
        let bundle = OpBundle::new(
            "agent".into(),
            "src/main.rs".into(),
            vec![AstOp::AddUse {
                path: "std::io".into(),
            }],
            "reason".into(),
        );
        let json = serde_json::to_string(&bundle).unwrap();
        let back = OpBundle::from_json(&json).unwrap();
        assert_eq!(bundle.content_hash, back.content_hash);
        assert_eq!(bundle.target_file, back.target_file);
    }
}
