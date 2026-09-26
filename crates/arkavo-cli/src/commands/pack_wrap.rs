//! `arkavo pack wrap` — seal an index under local key custody (KP-009).
//!
//! `pack seal` refuses a plaintext index (its labels say how sensitive the
//! corpus is), and nothing before this command turned `index.json` into the
//! `SealedBlob` a seal can accept. KAS custody is the passkey-login
//! dependency; until it lands, the payload key this command generates is
//! held locally, in a file the operator now owns like any other secret. The
//! wrapped-key record names that plainly (`urn:arkavo:local-custody`) so
//! nothing pretends a KAS was ever asked.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;

use arkavo_gguf_tdf::{GgufTdfError, PayloadKeyWrapper, WrappedKey};
use arkavo_knowledge_pack::{PackIndexes, seal_blob};
use arkavo_protocol::data_classification::SensitivityLevel;
use arkavo_protocol::taxonomy::TaxonomyMap;

/// Names the wrapped-key record produced by [`LocalCustodyWrapper`], so a
/// reader can tell at a glance that no KAS holds this component's key.
pub const LOCAL_CUSTODY_KAS: &str = "urn:arkavo:local-custody";

/// Records the payload key to a file the operator holds; the wrapped-key
/// field names local custody so no KAS is ever asked for it.
pub struct LocalCustodyWrapper {
    path: PathBuf,
}

impl LocalCustodyWrapper {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl PayloadKeyWrapper for LocalCustodyWrapper {
    fn wrap(&self, payload_key: &[u8; 32]) -> Result<WrappedKey, GgufTdfError> {
        // `create_new` is the whole point: a payload key silently overwriting
        // an existing file would either destroy a key still in use elsewhere
        // or, worse, be mistaken for one. Either way the failure belongs here,
        // not at the first place something later tries to open the blob.
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        // The mode is set on the `open` syscall itself, not with a `chmod`
        // afterward: a `set_permissions` call after `open` leaves a window
        // where the file exists at the process's default mode (typically
        // 0o644), which is a raw secret readable by anyone on the box for
        // however long that window lasts.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&self.path)?;
        file.write_all(payload_key)?;
        Ok(WrappedKey {
            kas_url: LOCAL_CUSTODY_KAS.to_string(),
            kid: None,
            // There is no wrap: the key sits in `self.path` in the clear, and
            // that file is the custody boundary. A base64 copy of the same
            // bytes here would just be a second place the secret lives.
            wrapped_key: String::new(),
        })
    }
}

struct Options {
    input: PathBuf,
    out: PathBuf,
    payload_key_out: PathBuf,
    taxonomy: Option<PathBuf>,
}

impl Options {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut input = None;
        let mut out = None;
        let mut payload_key_out = None;
        let mut taxonomy = None;
        let mut i = 0;
        while i < args.len() {
            let value = || -> Result<String, String> {
                args.get(i + 1)
                    .cloned()
                    .ok_or_else(|| format!("{} requires a value", args[i]))
            };
            match args[i].as_str() {
                "--in" => input = Some(PathBuf::from(value()?)),
                "--out" => out = Some(PathBuf::from(value()?)),
                "--payload-key-out" => payload_key_out = Some(PathBuf::from(value()?)),
                "--taxonomy" => taxonomy = Some(PathBuf::from(value()?)),
                other => return Err(format!("unknown option '{other}'")),
            }
            i += 2;
        }
        Ok(Self {
            input: input.ok_or("--in is required")?,
            out: out.ok_or("--out is required")?,
            payload_key_out: payload_key_out.ok_or("--payload-key-out is required")?,
            taxonomy,
        })
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let options = Options::parse(args)?;

    let taxonomy = match &options.taxonomy {
        Some(path) => {
            let json = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read taxonomy {}: {e}", path.display()))?;
            TaxonomyMap::from_json(&json).map_err(|e| format!("taxonomy is unusable: {e}"))?
        }
        None => TaxonomyMap::v1().clone(),
    };

    let plaintext = std::fs::read(&options.input)
        .map_err(|e| format!("cannot read {}: {e}", options.input.display()))?;
    let indexes: PackIndexes = serde_json::from_slice(&plaintext)
        .map_err(|e| format!("{} is not an index: {e}", options.input.display()))?;
    let sensitivity = content_sensitivity(&indexes);

    let attribute = wrap_attribute(&taxonomy, sensitivity)?;

    // A friendlier refusal than the generic I/O error `create_new` produces
    // inside `wrap`, which still enforces this atomically — this check only
    // improves the message an operator sees in the common case.
    if options.payload_key_out.exists() {
        return Err(format!(
            "the payload key file {} already exists; refusing to overwrite it",
            options.payload_key_out.display()
        ));
    }

    let wrapper = LocalCustodyWrapper::new(options.payload_key_out.clone());
    let sealed = seal_blob(
        &plaintext,
        &wrapper,
        std::slice::from_ref(&attribute),
        "application/json",
    )
    .map_err(|e| format!("cannot wrap {}: {e}", options.input.display()))?;
    let encoded = serde_json::to_vec(&sealed)
        .map_err(|e| format!("cannot serialize the wrapped index: {e}"))?;
    std::fs::write(&options.out, &encoded)
        .map_err(|e| format!("cannot write {}: {e}", options.out.display()))?;

    println!("Wrapped under: {attribute}");
    println!("Wrote {}", options.out.display());
    println!(
        "Wrote the payload key to {}. Keep it like a secret: anyone who \
         holds it can open this component, and KAS-backed custody is not \
         wired up yet.",
        options.payload_key_out.display()
    );
    Ok(())
}

/// Highest sensitivity across every tier the index carries.
///
/// The same high-water rule `load.rs`'s post-open check applies: inference may
/// only add restrictions, so the wrap has to cover whichever tier is most
/// sensitive rather than only the reference tier.
fn content_sensitivity(indexes: &PackIndexes) -> SensitivityLevel {
    let mut sensitivity = indexes.reference.max_sensitivity();
    if let Some(near) = &indexes.near {
        sensitivity = sensitivity.max(near.max_sensitivity());
    }
    if let Some(semantic) = &indexes.semantic {
        sensitivity = sensitivity.max(semantic.max_sensitivity());
    }
    sensitivity
}

/// The single attribute this index must be wrapped under, in the same
/// `<fqn>/<value>` shape `load.rs::policy_covers_ceiling` checks for.
fn wrap_attribute(taxonomy: &TaxonomyMap, sensitivity: SensitivityLevel) -> Result<String, String> {
    if sensitivity <= SensitivityLevel::Public {
        return Err(
            "the index holds only Public content; there is no clearance attribute to wrap it under"
                .to_string(),
        );
    }
    let requirement = taxonomy.clearance_requirement(sensitivity).ok_or_else(|| {
        format!(
            "taxonomy {} defines no clearance attribute for {sensitivity:?} content",
            taxonomy.version()
        )
    })?;
    Ok(format!("{}/{}", requirement.fqn, requirement.value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_fingerprint::{IndexKey, ReferenceIndex};
    use arkavo_gguf_tdf::PreResolvedKey;
    use arkavo_knowledge_pack::{blob::embedded_attributes, open_blob};
    use arkavo_protocol::data_classification::DataCategory;
    use rand::RngCore as _;

    fn random_secret() -> [u8; 32] {
        let mut secret = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut secret);
        secret
    }

    fn confidential_indexes() -> PackIndexes {
        let secret = random_secret();
        let key = IndexKey::derive(&secret, "default").expect("derive key");
        let mut builder = ReferenceIndex::builder(&key, "1.0.0");
        builder.add_document(
            &key,
            "the northwind acquisition closes in march with a hidden indemnity clause",
            DataCategory::Internal,
            SensitivityLevel::Confidential,
            "board",
        );
        PackIndexes {
            reference: builder.build(),
            near: None,
            semantic: None,
        }
    }

    #[test]
    fn a_wrapped_index_opens_with_the_recorded_key_and_passes_the_ceiling_check() {
        let dir = tempfile::tempdir().unwrap();
        let indexes = confidential_indexes();
        let plaintext = serde_json::to_vec(&indexes).unwrap();
        let in_path = dir.path().join("index.json");
        std::fs::write(&in_path, &plaintext).unwrap();
        let out_path = dir.path().join("index.json.tdf");
        let key_path = dir.path().join("payload.key");

        run(&[
            "--in".into(),
            in_path.to_str().unwrap().into(),
            "--out".into(),
            out_path.to_str().unwrap().into(),
            "--payload-key-out".into(),
            key_path.to_str().unwrap().into(),
        ])
        .expect("wrap succeeds");

        let sealed_json = std::fs::read(&out_path).unwrap();
        let sealed: arkavo_knowledge_pack::SealedBlob =
            serde_json::from_slice(&sealed_json).expect("wrapped output is a SealedBlob");

        let key_bytes = std::fs::read(&key_path).unwrap();
        assert_eq!(key_bytes.len(), 32, "the key file must hold raw 32 bytes");
        let mut payload_key = [0u8; 32];
        payload_key.copy_from_slice(&key_bytes);

        let opened = open_blob(&sealed, &PreResolvedKey::new(payload_key)).expect("opens");
        assert_eq!(opened.as_slice(), plaintext.as_slice());

        let attributes = embedded_attributes(&sealed.manifest).expect("policy parses");
        assert!(
            attributes
                .iter()
                .any(|a| a.ends_with("/clearance/confidential")),
            "expected a clearance/confidential attribute, got {attributes:?}"
        );
    }

    #[test]
    fn an_existing_key_file_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let indexes = confidential_indexes();
        let plaintext = serde_json::to_vec(&indexes).unwrap();
        let in_path = dir.path().join("index.json");
        std::fs::write(&in_path, &plaintext).unwrap();
        let out_path = dir.path().join("index.json.tdf");
        let key_path = dir.path().join("payload.key");
        std::fs::write(&key_path, [9u8; 32]).unwrap();

        let err = run(&[
            "--in".into(),
            in_path.to_str().unwrap().into(),
            "--out".into(),
            out_path.to_str().unwrap().into(),
            "--payload-key-out".into(),
            key_path.to_str().unwrap().into(),
        ])
        .unwrap_err();

        assert!(err.contains("already exists"), "{err}");
        assert!(!out_path.exists(), "no wrapped output should be written");
        // The pre-existing key must survive untouched.
        assert_eq!(std::fs::read(&key_path).unwrap(), vec![9u8; 32]);
    }

    /// `run`'s own `.exists()` check gives the friendlier message above, but
    /// the actual, TOCTOU-safe guarantee is `create_new` on the wrapper
    /// itself — checked directly here, independent of that pre-check.
    #[test]
    fn the_wrapper_itself_refuses_to_overwrite_via_create_new() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("payload.key");
        std::fs::write(&key_path, [9u8; 32]).unwrap();
        let wrapper = LocalCustodyWrapper::new(key_path.clone());

        let err = wrapper.wrap(&[1u8; 32]).unwrap_err();

        assert!(matches!(err, GgufTdfError::Io(_)), "{err:?}");
        assert_eq!(std::fs::read(&key_path).unwrap(), vec![9u8; 32]);
    }

    #[cfg(unix)]
    #[test]
    fn the_key_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let indexes = confidential_indexes();
        let plaintext = serde_json::to_vec(&indexes).unwrap();
        let in_path = dir.path().join("index.json");
        std::fs::write(&in_path, &plaintext).unwrap();
        let out_path = dir.path().join("index.json.tdf");
        let key_path = dir.path().join("payload.key");

        run(&[
            "--in".into(),
            in_path.to_str().unwrap().into(),
            "--out".into(),
            out_path.to_str().unwrap().into(),
            "--payload-key-out".into(),
            key_path.to_str().unwrap().into(),
        ])
        .expect("wrap succeeds");

        let mode = std::fs::metadata(&key_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn public_only_content_is_refused_with_a_clear_reason() {
        let secret = random_secret();
        let key = IndexKey::derive(&secret, "default").expect("derive key");
        let mut builder = ReferenceIndex::builder(&key, "1.0.0");
        builder.add_document(
            &key,
            "a public press release about the quarterly launch",
            DataCategory::Public,
            SensitivityLevel::Public,
            "press",
        );
        let indexes = PackIndexes {
            reference: builder.build(),
            near: None,
            semantic: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let plaintext = serde_json::to_vec(&indexes).unwrap();
        let in_path = dir.path().join("index.json");
        std::fs::write(&in_path, &plaintext).unwrap();

        let err = run(&[
            "--in".into(),
            in_path.to_str().unwrap().into(),
            "--out".into(),
            dir.path().join("out.tdf").to_str().unwrap().into(),
            "--payload-key-out".into(),
            dir.path().join("key.bin").to_str().unwrap().into(),
        ])
        .unwrap_err();

        assert!(
            err.contains("does not need wrapping") || err.contains("Public content"),
            "{err}"
        );
    }
}
