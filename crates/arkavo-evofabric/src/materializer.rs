use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::time::Duration;

use crate::canonical_store::CanonicalStore;
use crate::{Error, Result, RustOp, render_file};

/// Deterministic pipeline from canonical IR to compiled artifacts.
///
/// Retrieves source from the canonical store, renders it, and compiles
/// it in an isolated workspace to produce verifiable build receipts.
pub struct Materializer {
    store: CanonicalStore,
    project_root: PathBuf,
}

/// Receipt proving a version was compiled and tested successfully.
#[derive(Debug, Clone)]
pub struct BuildReceipt {
    pub source_hash: [u8; 32],
    pub compile_ok: bool,
    pub test_ok: bool,
    pub duration: Duration,
    pub rendered_at: DateTime<Utc>,
}

impl Materializer {
    /// Create a new materializer with a store and project root.
    pub fn new(store: CanonicalStore, project_root: PathBuf) -> Self {
        Self {
            store,
            project_root,
        }
    }

    /// Get a reference to the canonical store.
    pub fn store(&self) -> &CanonicalStore {
        &self.store
    }

    /// Get a mutable reference to the canonical store.
    pub fn store_mut(&mut self) -> &mut CanonicalStore {
        &mut self.store
    }

    /// Get the project root path.
    pub fn project_root(&self) -> &PathBuf {
        &self.project_root
    }

    /// Verify that re-rendering a stored version produces identical output.
    ///
    /// This checks determinism: parse the stored source, render it via
    /// prettyplease, and compare. If they differ, the store is inconsistent.
    pub fn verify_determinism(&self, hash: &[u8; 32]) -> Result<bool> {
        let entry = self
            .store
            .get(hash)
            .ok_or_else(|| Error::VersionNotFound(hex::encode(hash)))?;

        let file = RustOp::parse(&entry.rendered_source)?;
        let re_rendered = render_file(&file);

        Ok(re_rendered == entry.rendered_source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical_store::CanonicalStore;
    use arkavo_test_macros::spec;

    #[spec("EVO-008")]
    #[test]
    fn verify_determinism_valid_source() {
        let mut store = CanonicalStore::new();
        let source = "fn main() {\n    println!(\"hello\");\n}\n";
        // Store the prettyplease-rendered version
        let file = RustOp::parse(source).unwrap();
        let rendered = render_file(&file);
        let hash = store.store(&rendered, &[], None);

        let mat = Materializer::new(store, PathBuf::from("/tmp"));
        assert!(mat.verify_determinism(&hash).unwrap());
    }

    #[spec("EVO-008")]
    #[test]
    fn verify_determinism_missing_version() {
        let store = CanonicalStore::new();
        let mat = Materializer::new(store, PathBuf::from("/tmp"));
        let err = mat.verify_determinism(&[0xFF; 32]).unwrap_err();
        assert!(err.to_string().contains("version not found"));
    }

    #[spec("EVO-008")]
    #[test]
    fn store_access() {
        let mut store = CanonicalStore::new();
        store.store("fn x() {}", &[], None);
        let mat = Materializer::new(store, PathBuf::from("/project"));
        assert_eq!(mat.store().len(), 1);
        assert_eq!(mat.project_root(), &PathBuf::from("/project"));
    }

    #[spec("EVO-008")]
    #[test]
    fn store_mut_access() {
        let store = CanonicalStore::new();
        let mut mat = Materializer::new(store, PathBuf::from("/project"));
        mat.store_mut().store("fn y() {}", &[], None);
        assert_eq!(mat.store().len(), 1);
    }

    #[spec("EVO-008")]
    #[test]
    fn determinism_after_reparse() {
        let mut store = CanonicalStore::new();
        // Intentionally poorly formatted source
        let source = "fn   foo  (  )  {  let x=1;  }";
        let file = RustOp::parse(source).unwrap();
        let rendered = render_file(&file);
        let hash = store.store(&rendered, &[], None);

        let mat = Materializer::new(store, PathBuf::from("/tmp"));
        // prettyplease output should be stable across re-parses
        assert!(mat.verify_determinism(&hash).unwrap());
    }

    #[spec("EVO-008")]
    #[test]
    fn build_receipt_fields() {
        let receipt = BuildReceipt {
            source_hash: [1u8; 32],
            compile_ok: true,
            test_ok: true,
            duration: Duration::from_secs(5),
            rendered_at: Utc::now(),
        };
        assert!(receipt.compile_ok);
        assert!(receipt.test_ok);
        assert_eq!(receipt.duration.as_secs(), 5);
    }

    #[spec("EVO-008")]
    #[test]
    fn verify_complex_source() {
        let mut store = CanonicalStore::new();
        let source = r#"
            struct Config {
                name: String,
                value: i32,
            }
            impl Config {
                fn new(name: String) -> Self {
                    Self { name, value: 0 }
                }
            }
        "#;
        let file = RustOp::parse(source).unwrap();
        let rendered = render_file(&file);
        let hash = store.store(&rendered, &[], None);

        let mat = Materializer::new(store, PathBuf::from("/tmp"));
        assert!(mat.verify_determinism(&hash).unwrap());
    }

    #[spec("EVO-008")]
    #[test]
    fn verify_source_with_attributes() {
        let mut store = CanonicalStore::new();
        let source = "#[inline]\n#[must_use]\nfn compute() -> i32 { 42 }\n";
        let file = RustOp::parse(source).unwrap();
        let rendered = render_file(&file);
        let hash = store.store(&rendered, &[], None);

        let mat = Materializer::new(store, PathBuf::from("/tmp"));
        assert!(mat.verify_determinism(&hash).unwrap());
    }
}
