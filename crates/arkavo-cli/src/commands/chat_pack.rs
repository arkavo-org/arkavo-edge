//! `arkavo chat --pack`: provision a sealed knowledge pack into the release
//! policy before the session's first completion.
//!
//! Until this entry point a pack could be sealed, verified and loaded, and
//! nothing served under it. The order below is the trust order: nothing from
//! the pack is read — not even which embedder it wants — until its signature
//! has checked out against the operator's anchor.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arkavo_fingerprint::{Embedder, IndexKey};
use arkavo_gguf_tdf::PreResolvedKey;
use arkavo_knowledge_pack::{semantic_embedder_record, verify_pack};

use crate::commands::pack_seal::read_anchor;
use crate::sentinel_embedder::{LlamaEmbedder, fetch_embedder};
use crate::sentinel_wiring::{self, SentinelRuntime};

/// Where a sealed pack and the keys that open it live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackArgs {
    pub pack: PathBuf,
    pub anchor: PathBuf,
    pub index_key: PathBuf,
    pub index_id: String,
    pub payload_key: PathBuf,
}

/// Pull the pack flags out of `chat`'s arguments, ignoring the rest.
///
/// A pack flag given without `--pack`, or `--pack` without every key it
/// needs, is refused rather than dropped: an operator who asked for a pack
/// and silently got none would believe a policy was enforced that is not.
pub fn parse_pack_args(args: &[String]) -> Result<Option<PackArgs>, String> {
    let mut pack = None;
    let mut anchor = None;
    let mut index_key = None;
    let mut index_id = None;
    let mut payload_key = None;
    let mut i = 0;
    while i < args.len() {
        let slot = match args[i].as_str() {
            "--pack" => &mut pack,
            "--anchor" => &mut anchor,
            "--index-key" => &mut index_key,
            "--index-id" => &mut index_id,
            "--payload-key" => &mut payload_key,
            _ => {
                i += 1;
                continue;
            }
        };
        let value = args
            .get(i + 1)
            .ok_or_else(|| format!("{} requires a value", args[i]))?;
        *slot = Some(value.clone());
        i += 2;
    }

    let Some(pack) = pack else {
        let given = [
            ("--anchor", &anchor),
            ("--index-key", &index_key),
            ("--index-id", &index_id),
            ("--payload-key", &payload_key),
        ]
        .into_iter()
        .find(|(_, v)| v.is_some());
        return match given {
            Some((flag, _)) => Err(format!("{flag} is only meaningful with --pack")),
            None => Ok(None),
        };
    };
    let required = |value: Option<String>, flag: &str| {
        value
            .map(PathBuf::from)
            .ok_or_else(|| format!("--pack requires {flag}"))
    };
    Ok(Some(PackArgs {
        pack: PathBuf::from(pack),
        anchor: required(anchor, "--anchor")?,
        index_key: required(index_key, "--index-key")?,
        index_id: index_id.unwrap_or_else(|| "default".to_string()),
        payload_key: required(payload_key, "--payload-key")?,
    }))
}

/// Verify, load and provision the pack; returns its inventory for the
/// operator.
pub async fn provision_from_pack(args: &PackArgs) -> Result<String, String> {
    let anchor = read_anchor(&args.anchor)?;
    let verified = verify_pack(&args.pack, Some(&anchor)).map_err(|e| e.to_string())?;

    let embedder: Option<Arc<dyn Embedder>> =
        match semantic_embedder_record(&verified.manifest.thresholds) {
            Some(record) => {
                let path = fetch_embedder(&record).await?;
                Some(Arc::new(LlamaEmbedder::load(&path, record.pooling)?))
            }
            None => None,
        };

    let secret = std::fs::read(&args.index_key).map_err(|e| {
        format!(
            "cannot read the tenant index key {}: {e}",
            args.index_key.display()
        )
    })?;
    let index_key = Arc::new(
        IndexKey::derive(&secret, &args.index_id)
            .map_err(|e| format!("the tenant index key is unusable: {e}"))?,
    );
    let payload_key = read_payload_key(&args.payload_key)?;

    let runtime = SentinelRuntime::from_pack(
        &verified,
        Some(&index_key),
        &PreResolvedKey::new(payload_key),
        embedder,
    )
    .map_err(|e| e.to_string())?;
    let inventory = runtime.inventory.clone();
    sentinel_wiring::provision(runtime)?;
    Ok(inventory)
}

/// Read a raw 32-byte payload key.
///
/// Read into a fixed buffer rather than a growable one, so a wrong file is
/// refused without being slurped whole and the key never lands in a heap
/// allocation that outlives this call unzeroed.
fn read_payload_key(path: &Path) -> Result<[u8; 32], String> {
    let unreadable =
        |e: std::io::Error| format!("cannot read the payload key {}: {e}", path.display());
    let mut file = std::fs::File::open(path).map_err(unreadable)?;
    let mut key = [0u8; 32];
    file.read_exact(&mut key)
        .map_err(|_| "the payload key file must be exactly 32 bytes".to_string())?;
    if file.read(&mut [0u8; 1]).map_err(unreadable)? != 0 {
        return Err("the payload key file must be exactly 32 bytes".to_string());
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_without_its_keys_is_refused() {
        let err = parse_pack_args(&["--pack".into(), "p".into(), "--anchor".into(), "a".into()])
            .unwrap_err();
        assert!(err.contains("--index-key"));
    }

    #[test]
    fn no_pack_flags_means_no_pack() {
        assert!(
            parse_pack_args(&["--prompt".into(), "hi".into()])
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn index_id_defaults() {
        let a = parse_pack_args(
            &[
                "--pack",
                "p",
                "--anchor",
                "a",
                "--index-key",
                "k",
                "--payload-key",
                "pk",
            ]
            .map(String::from),
        )
        .unwrap()
        .unwrap();
        assert_eq!(a.index_id, "default");
    }

    #[test]
    fn a_key_flag_without_a_pack_is_refused() {
        let err = parse_pack_args(&["--payload-key", "pk"].map(String::from)).unwrap_err();
        assert!(err.contains("--payload-key"), "{err}");
    }

    #[test]
    fn a_pack_flag_without_its_value_is_refused() {
        let err = parse_pack_args(&["--prompt", "hi", "--pack"].map(String::from)).unwrap_err();
        assert!(err.contains("--pack requires a value"), "{err}");
    }

    #[test]
    fn a_payload_key_must_be_exactly_32_bytes() {
        let dir = tempfile::tempdir().unwrap();
        for (len, ok) in [(31, false), (32, true), (33, false)] {
            let path = dir.path().join(format!("key-{len}"));
            std::fs::write(&path, vec![7u8; len]).unwrap();
            let read = read_payload_key(&path);
            assert_eq!(read.is_ok(), ok, "{len} bytes: {read:?}");
            if let Err(e) = read {
                assert!(e.contains("exactly 32 bytes"), "{e}");
            }
        }
        assert_eq!(
            read_payload_key(&dir.path().join("key-32")).unwrap(),
            [7u8; 32]
        );
    }
}
