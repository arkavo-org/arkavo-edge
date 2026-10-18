//! The agent's workload key: the one key its identity rests on.
//!
//! An agent's account is this key (#766). Its `did:key` names the agent to
//! peers and to the identity provider, and a new key is a new agent. On a Mac
//! with a Secure Enclave the enclave makes and keeps the private key, and the
//! file holds only the enclave's wrapped form of it, which no other Mac can
//! use. Elsewhere the key is a software P-256 key and the file says so,
//! because from outside the two are indistinguishable.

use crate::{DeviceIdentityError, Result};
use arkavo_crypto::{P256SigningKeypair, P256VerifyingKey};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

const WORKLOAD_KEY_FILENAME: &str = "workload_key";

// The agent slot beside this file holds a bare 32-byte Ed25519 seed, the
// same size as a P-256 scalar, so a key file is never read without its
// format named in it.
const FORMAT: &str = "arkavo-workload-key/1";

/// Where the private half of a workload key lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyStore {
    SecureEnclave,
    Software,
}

impl fmt::Display for KeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SecureEnclave => "Secure Enclave",
            Self::Software => "software key",
        })
    }
}

#[derive(Serialize, Deserialize)]
struct Record {
    format: String,
    store: KeyStore,
    key: String,
}

enum Secret {
    #[cfg(target_os = "macos")]
    Enclave(arkavo_secure_enclave::WrappedKey),
    Software(P256SigningKeypair),
}

/// The agent's persistent P-256 identity key.
pub struct WorkloadKey {
    store: KeyStore,
    secret: Secret,
    public: P256VerifyingKey,
    did: String,
}

impl WorkloadKey {
    fn new(store: KeyStore, secret: Secret, public: P256VerifyingKey) -> Self {
        let did = public.to_did_key();
        Self {
            store,
            secret,
            public,
            did,
        }
    }

    /// The agent's `did:key` (`did:key:zDn…`).
    pub fn did(&self) -> &str {
        &self.did
    }

    /// The DID URL of the key itself, the form a JWS `kid` and a DID
    /// document use: a `did:key` has one verification method, named by the
    /// DID's own multibase value.
    pub fn key_id(&self) -> String {
        let multibase = self.did.trim_start_matches("did:key:");
        format!("{}#{multibase}", self.did)
    }

    pub fn store(&self) -> KeyStore {
        self.store
    }

    pub fn public_key(&self) -> &P256VerifyingKey {
        &self.public
    }

    /// ECDSA P-256 with SHA-256 over `message`, as the 64-byte `r || s`
    /// that JWS ES256 and COSE carry.
    pub fn sign(&self, message: &[u8]) -> Result<[u8; 64]> {
        match &self.secret {
            #[cfg(target_os = "macos")]
            Secret::Enclave(wrapped) => wrapped
                .sign(message)
                .map_err(|e| DeviceIdentityError::WorkloadKey(e.to_string())),
            Secret::Software(keypair) => Ok(keypair.sign_p1363(message)),
        }
    }
}

impl fmt::Debug for WorkloadKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkloadKey")
            .field("did", &self.did)
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

/// The workload key of the agent named `agent`, made on first use. Each
/// name is its own account, so two agents one user runs on one machine are
/// two agents to their peers. On a Mac whose Secure Enclave this process can
/// use, the key is made there; if the enclave is present but refuses, this
/// fails rather than settle for a software key that would pass for an
/// enclave one.
pub fn load_or_create(agent: &str) -> Result<WorkloadKey> {
    load_or_create_at(&crate::agent_dir::agent_dir(agent)?.join(WORKLOAD_KEY_FILENAME))
}

/// [`load_or_create`] for a key file at `path` instead of the agent's own
/// directory.
pub fn load_or_create_at(path: &Path) -> Result<WorkloadKey> {
    if let Some(key) = load(path)? {
        return Ok(key);
    }
    if let Some(dir) = path.parent() {
        crate::private_file::create_private_dir(dir)?;
    }
    let (record, key) = create()?;
    if crate::private_file::create_private(path, &record)? {
        return Ok(key);
    }
    // Another process started at the same moment; its key is the agent's.
    load(path)?.ok_or_else(|| {
        DeviceIdentityError::WorkloadKey(format!("{} vanished as it was made", path.display()))
    })
}

fn create() -> Result<(Vec<u8>, WorkloadKey)> {
    #[cfg(target_os = "macos")]
    if arkavo_secure_enclave::is_available() {
        let wrapped = arkavo_secure_enclave::WrappedKey::create()
            .map_err(|e| DeviceIdentityError::WorkloadKey(e.to_string()))?;
        let public = enclave_public_key(&wrapped)
            .map_err(|e| DeviceIdentityError::WorkloadKey(e.to_string()))?;
        let record = encode(KeyStore::SecureEnclave, wrapped.as_bytes())?;
        let key = WorkloadKey::new(KeyStore::SecureEnclave, Secret::Enclave(wrapped), public);
        return Ok((record, key));
    }
    software()
}

fn software() -> Result<(Vec<u8>, WorkloadKey)> {
    let keypair = P256SigningKeypair::generate();
    let record = encode(KeyStore::Software, &keypair.to_bytes())?;
    let public = keypair.public_key();
    Ok((
        record,
        WorkloadKey::new(KeyStore::Software, Secret::Software(keypair), public),
    ))
}

#[cfg(target_os = "macos")]
fn enclave_public_key(
    wrapped: &arkavo_secure_enclave::WrappedKey,
) -> std::result::Result<P256VerifyingKey, String> {
    let point = wrapped.public_key().map_err(|e| e.to_string())?;
    P256VerifyingKey::from_sec1_bytes(&point).map_err(|e| e.to_string())
}

fn encode(store: KeyStore, key: &[u8]) -> Result<Vec<u8>> {
    serde_json::to_vec(&Record {
        format: FORMAT.to_string(),
        store,
        key: STANDARD.encode(key),
    })
    .map_err(|e| DeviceIdentityError::WorkloadKey(e.to_string()))
}

fn load(path: &Path) -> Result<Option<WorkloadKey>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(DeviceIdentityError::Storage(format!(
                "Failed to read {}: {e}",
                path.display()
            )));
        }
    };
    // The file is the agent's account, so it is never replaced on the
    // agent's behalf; the operator decides whether to start a new agent.
    let refuse = |why: &str| {
        DeviceIdentityError::WorkloadKey(format!(
            "{} {why}; move it aside to start a new agent, under a new name: peers know this name by its old key",
            path.display()
        ))
    };
    let record: Record =
        serde_json::from_slice(&bytes).map_err(|_| refuse("is not a workload key"))?;
    if record.format != FORMAT {
        return Err(refuse(&format!(
            "is a {} key, which this version does not read",
            record.format
        )));
    }
    let key = STANDARD
        .decode(&record.key)
        .map_err(|_| refuse("is damaged"))?;
    match record.store {
        KeyStore::Software => {
            if key.len() != 32 {
                return Err(refuse("is damaged"));
            }
            let keypair = P256SigningKeypair::from_bytes(&key).map_err(|_| refuse("is damaged"))?;
            let public = keypair.public_key();
            Ok(Some(WorkloadKey::new(
                KeyStore::Software,
                Secret::Software(keypair),
                public,
            )))
        }
        #[cfg(target_os = "macos")]
        KeyStore::SecureEnclave => {
            let wrapped = arkavo_secure_enclave::WrappedKey::from_bytes(key);
            let public = enclave_public_key(&wrapped)
                .map_err(|e| refuse(&format!("cannot be used: {e}")))?;
            Ok(Some(WorkloadKey::new(
                KeyStore::SecureEnclave,
                Secret::Enclave(wrapped),
                public,
            )))
        }
        #[cfg(not(target_os = "macos"))]
        KeyStore::SecureEnclave => Err(refuse("is held by a Mac's Secure Enclave")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = crate::test_storage_dir().unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let _ = std::fs::remove_file(&path);
        path
    }

    fn verifies(key: &WorkloadKey, message: &[u8]) -> bool {
        let signature = key.sign(message).unwrap();
        key.public_key().verify(message, &signature).is_ok()
    }

    #[spec("DEVICE-008")]
    #[test]
    fn a_software_key_reloads_as_the_same_agent() {
        let path = scratch("software_reload");
        let (record, made) = software().unwrap();
        assert!(crate::private_file::create_private(&path, &record).unwrap());

        let loaded = load(&path).unwrap().unwrap();
        assert_eq!(loaded.did(), made.did());
        assert_eq!(loaded.store(), KeyStore::Software);
        assert!(loaded.did().starts_with("did:key:zDn"), "{}", loaded.did());
        assert_eq!(
            loaded.key_id(),
            format!("{}#{}", loaded.did(), &loaded.did()["did:key:".len()..])
        );
        assert!(verifies(&loaded, b"agent card"));
    }

    #[spec("DEVICE-008")]
    #[test]
    fn two_processes_starting_at_once_settle_on_one_key() {
        let path = scratch("concurrent");
        let dids: Vec<String> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..4)
                .map(|_| scope.spawn(|| load_or_create_at(&path).unwrap().did().to_string()))
                .collect();
            workers.into_iter().map(|w| w.join().unwrap()).collect()
        });
        assert!(dids.windows(2).all(|pair| pair[0] == pair[1]), "{dids:?}");
        assert_eq!(load_or_create_at(&path).unwrap().did(), dids[0]);
    }

    #[cfg(unix)]
    #[spec("DEVICE-008")]
    #[test]
    fn the_key_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = scratch("mode");
        load_or_create_at(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// The old agent slot is a bare 32-byte seed; read as a key it would
    /// silently become a different agent.
    #[spec("DEVICE-009")]
    #[test]
    fn a_file_that_is_not_a_workload_key_is_refused_and_kept() {
        let path = scratch("legacy_seed");
        std::fs::write(&path, [9u8; 32]).unwrap();
        let err = load_or_create_at(&path).unwrap_err().to_string();
        assert!(err.contains("is not a workload key"), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), [9u8; 32]);
    }

    #[spec("DEVICE-009")]
    #[test]
    fn a_newer_format_is_refused_and_kept() {
        let path = scratch("newer");
        let newer = br#"{"format":"arkavo-workload-key/2","store":"software","key":"AA=="}"#;
        std::fs::write(&path, newer).unwrap();
        let err = load_or_create_at(&path).unwrap_err().to_string();
        assert!(err.contains("arkavo-workload-key/2"), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), newer);
    }

    /// A wrapped key copied from another Mac must not be replaced by a new
    /// key: that would be a different agent under the same file.
    #[spec("DEVICE-009")]
    #[test]
    fn an_enclave_key_from_another_mac_is_refused_and_kept() {
        let path = scratch("foreign_enclave");
        let record = encode(KeyStore::SecureEnclave, &[7u8; 324]).unwrap();
        std::fs::write(&path, &record).unwrap();
        let err = load_or_create_at(&path).unwrap_err().to_string();
        assert!(err.contains("move it aside"), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), record);
    }

    fn agents_dir() -> std::path::PathBuf {
        crate::keypair::slot_path(crate::agent_dir::AGENTS_DIR).unwrap()
    }

    #[spec("DEVICE-010")]
    #[test]
    fn each_agent_name_is_its_own_account() {
        let planner = load_or_create("planner-own-account").unwrap();
        let coder = load_or_create("coder-own-account").unwrap();
        assert_ne!(planner.did(), coder.did());
        assert_eq!(
            load_or_create("planner-own-account").unwrap().did(),
            planner.did()
        );
    }

    #[spec("DEVICE-010")]
    #[test]
    fn names_that_differ_in_case_alone_are_one_agent() {
        let upper = load_or_create("Planner-Case").unwrap();
        assert_eq!(load_or_create("planner-case").unwrap().did(), upper.did());
    }

    #[spec("DEVICE-010")]
    #[test]
    fn a_name_with_path_characters_keeps_its_key_inside_the_agents_directory() {
        load_or_create("../escape-agents-dir").unwrap();
        let inside = agents_dir().join("%2e%2e%2fescape-agents-dir");
        assert!(inside.join(WORKLOAD_KEY_FILENAME).is_file());
        assert!(!agents_dir().join("../escape-agents-dir").exists());
    }

    #[spec("DEVICE-010")]
    #[test]
    fn an_agent_without_a_name_has_no_key() {
        let err = load_or_create("").unwrap_err().to_string();
        assert!(err.contains("needs a name"), "{err}");
    }

    #[cfg(unix)]
    #[spec("DEVICE-010")]
    #[test]
    fn an_agent_directory_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        load_or_create("private-dir").unwrap();
        let mode = std::fs::metadata(agents_dir().join("private-dir"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[cfg(target_os = "macos")]
    #[spec("DEVICE-008")]
    #[test]
    #[ignore = "requires a Secure Enclave"]
    fn on_a_mac_with_a_secure_enclave_the_key_is_made_there() {
        assert!(arkavo_secure_enclave::is_available());
        let path = scratch("enclave");
        let made = load_or_create_at(&path).unwrap();
        assert_eq!(made.store(), KeyStore::SecureEnclave);
        let reloaded = load_or_create_at(&path).unwrap();
        assert_eq!(reloaded.did(), made.did());
        assert!(verifies(&reloaded, b"agent card"));
        let stored = std::fs::read_to_string(&path).unwrap();
        assert!(stored.contains(r#""store":"secure-enclave""#), "{stored}");
    }
}
