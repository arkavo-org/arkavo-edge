//! Sealed knowledge packs.
//!
//! A pack is several separately wrapped components — knowledge adapters, a DLP
//! sentinel, a keyed reference index — bound by one signed manifest. Separately
//! wrapped is the load-bearing part: an egress node that enforces
//! classification needs the sentinel and the index and must never be able to
//! open the knowledge model, and that is a property of how the components are
//! keyed rather than of what anyone chose to ship.

pub mod assemble;
pub mod blob;
pub mod load;
pub mod manifest;
pub mod selection;
pub mod sign;
pub mod verify;

pub use assemble::{AssembleError, PackBuilder};
pub use blob::{MAX_BLOB_BYTES, SealedBlob, open_blob, seal_blob};
pub use load::{LoadError, LoadedPack, PackIndexes, load_pack};
pub use manifest::{
    ComponentRecord, Lineage, ManifestError, PACK_FORMAT_VERSION, PACK_MANIFEST_FILE,
    PACK_SIGNATURE_FILE, PackManifest, digest_of,
};
pub use selection::{Entitlements, Selection, SelectionError, select_adapters};
pub use sign::{
    SignatureError, decode_signature, encode_signature, sign_manifest, verify_manifest,
};
pub use verify::{VerifiedPack, VerifyError, verify_pack};
