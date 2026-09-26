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
use arkavo_gguf_tdf::{ComponentRole, PreResolvedKey};
use arkavo_knowledge_pack::{VerifiedPack, semantic_embedder_record, verify_pack};
use zeroize::Zeroizing;

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

/// Releases a provisioned embedder's llama.cpp resources when dropped.
///
/// The pack's policy outlives every session (provisioning is once, upward
/// only), so the embedder inside it is never dropped; left alone, its Metal
/// buffers are still resident when ggml's static destructors run, and the
/// process aborts on exit. The session holds this guard and drops it on
/// every way out, so the native half goes first. The policy stays
/// installed: anything it still inspects after that is a gap, and held.
pub struct NativeRelease(Option<Arc<LlamaEmbedder>>);

impl NativeRelease {
    pub fn new(embedder: Option<Arc<LlamaEmbedder>>) -> Self {
        Self(embedder)
    }
}

impl Drop for NativeRelease {
    fn drop(&mut self) {
        if let Some(embedder) = &self.0 {
            embedder.unload();
        }
    }
}

/// Verify, load and provision the pack; returns its inventory for the
/// operator and the guard that releases its embedder when the session ends.
pub async fn provision_from_pack(args: &PackArgs) -> Result<(String, NativeRelease), String> {
    let local = open_local(args)?;

    let loaded: Option<Arc<LlamaEmbedder>> =
        match semantic_embedder_record(&local.verified.manifest.thresholds) {
            Some(record) => {
                let path = fetch_embedder(&record).await?;
                Some(Arc::new(LlamaEmbedder::load(&path, record.pooling)?))
            }
            None => None,
        };
    // Taken before anything below can fail, so an early return still frees
    // the embedder rather than leaving it for the exit-time destructors.
    let release = NativeRelease::new(loaded.clone());
    let embedder = loaded.map(|e| -> Arc<dyn Embedder> { e });

    let runtime = SentinelRuntime::from_pack(
        &local.verified,
        Some(&local.index_key),
        &PreResolvedKey::new(*local.payload_key),
        embedder,
    )
    .map_err(|e| e.to_string())?;
    let inventory = runtime.inventory.clone();
    sentinel_wiring::provision(runtime)?;
    Ok((inventory, release))
}

/// Everything `provision_from_pack` needs that this node already holds.
struct LocalPack {
    verified: VerifiedPack,
    index_key: Arc<IndexKey>,
    payload_key: Zeroizing<[u8; 32]>,
}

/// Verify the pack and read the operator's keys, all before the embedder
/// fetch — a download of hundreds of megabytes that a wrong path or a pack
/// this node cannot serve should never have to wait for.
fn open_local(args: &PackArgs) -> Result<LocalPack, String> {
    let anchor = read_anchor(&args.anchor)?;
    let verified = verify_pack(&args.pack, Some(&anchor)).map_err(|e| e.to_string())?;
    require_index(&verified)?;

    // Only the derived key outlives this call; the secret it came from is
    // wiped when this function returns, whether or not derivation succeeds.
    let secret = Zeroizing::new(std::fs::read(&args.index_key).map_err(|e| {
        format!(
            "cannot read the tenant index key {}: {e}",
            args.index_key.display()
        )
    })?);
    let index_key = Arc::new(
        IndexKey::derive(&secret, &args.index_id)
            .map_err(|e| format!("the tenant index key is unusable: {e}"))?,
    );
    let payload_key = read_payload_key(&args.payload_key)?;
    Ok(LocalPack {
        verified,
        index_key,
        payload_key,
    })
}

/// `--pack` requires index and payload keys, so the operator asked for the
/// pack's index. Loading a pack in general treats a component this node was
/// never sent as an absence, and would quietly provision the pattern tier
/// alone — a far weaker policy than the one the operator named.
fn require_index(verified: &VerifiedPack) -> Result<(), String> {
    let pack_id = &verified.manifest.pack_id;
    let Some(record) = verified.manifest.role(&ComponentRole::Index) else {
        return Err(format!(
            "pack {pack_id} has no index component; --index-key and --payload-key open one"
        ));
    };
    if !verified.holds(&record.file) {
        return Err(format!(
            "pack {pack_id} lists index component {} but it is not held on this node",
            record.file
        ));
    }
    Ok(())
}

/// Read a raw 32-byte payload key.
///
/// Read straight into a fixed, zeroizing buffer: a wrong file is refused
/// without being read whole, and this function's copy of the key is wiped
/// when the caller drops it. `PreResolvedKey::new` takes the key by value, so
/// the one copy made for that call is a stack temporary nothing here can
/// wipe; the key it then holds is zeroized by `PreResolvedKey` itself.
fn read_payload_key(path: &Path) -> Result<Zeroizing<[u8; 32]>, String> {
    let unreadable =
        |e: std::io::Error| format!("cannot read the payload key {}: {e}", path.display());
    let mut file = std::fs::File::open(path).map_err(unreadable)?;
    let mut key = Zeroizing::new([0u8; 32]);
    file.read_exact(key.as_mut())
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
            *read_payload_key(&dir.path().join("key-32")).unwrap(),
            [7u8; 32]
        );
    }
}
