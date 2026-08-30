//! Error codes for `gguf-tdf/1` (spec §14).
//!
//! Every variant fails closed: no plaintext weights are emitted, no sibling
//! `.gguf` is opened, and scratch buffers are zeroized before the error
//! propagates.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GgufTdfError {
    #[error("GGUFTDF_NOT_ZIP: archive is not a ZIP file")]
    NotZip,

    #[error("GGUFTDF_NO_MANIFEST: neither 0.manifest.json nor manifest.json is present")]
    NoManifest,

    #[error("GGUFTDF_UNSUPPORTED_PROFILE: gguf.profile is {0:?}, expected gguf-tdf/1")]
    UnsupportedProfile(String),

    #[error("GGUFTDF_NOT_GGUF: first four bytes are not 47 47 55 46")]
    NotGguf,

    #[error("GGUFTDF_UNSUPPORTED_GGUF_VERSION: GGUF version {0}, expected little-endian 3")]
    UnsupportedGgufVersion(u32),

    #[error("GGUFTDF_UNSUPPORTED_ENDIAN: file appears to be big-endian GGUF")]
    UnsupportedEndian,

    #[error("GGUFTDF_BAD_ALIGN: alignment {0} is not a power of two >= 8")]
    BadAlign(u64),

    #[error("GGUFTDF_BAD_MAX_SEGMENT: maxSegment {0} is not a multiple of alignment")]
    BadMaxSegment(u64),

    #[error("GGUFTDF_BAD_HEADER: {0}")]
    BadHeader(String),

    #[error("GGUFTDF_BAD_TENSOR: {0}")]
    BadTensor(String),

    #[error("GGUFTDF_OVERLAP: tensors overlap in the virtual file")]
    Overlap,

    #[error("GGUFTDF_BAD_INDEX: {0}")]
    BadIndex(String),

    #[error("GGUFTDF_SIZE_MISMATCH: virtualSize disagrees with the sum of segment plain sizes")]
    SizeMismatch,

    #[error("GGUFTDF_KAS_DENIED: {0}")]
    KasDenied(String),

    #[error("GGUFTDF_TAG_MISMATCH: AES-GCM tag mismatch or wrong member length")]
    TagMismatch,

    #[error("GGUFTDF_ROOT_MISMATCH: root signature mismatch")]
    RootMismatch,

    #[error("GGUFTDF_READ_AT_ZERO: callback returned 0 bytes mid-tensor")]
    ReadAtZero,

    #[error("GGUFTDF_MTMD_UNSUPPORTED: mmproj load requires a callback-capable mtmd API")]
    MtmdUnsupported,

    #[error("GGUFTDF_SIBLING_REFUSED: refusing to load a sibling .gguf when a .gguf.tdf exists")]
    SiblingRefused,

    #[error("GGUFTDF_PAYLOAD_FORBIDDEN: archive contains 0.payload alongside profile members")]
    PayloadForbidden,

    #[error("GGUFTDF_CRYPTO: {0}")]
    Crypto(String),

    #[error("GGUFTDF_BAD_INDEX: I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl GgufTdfError {
    /// The spec §14 code, for structured logging and HTTP status mapping.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotZip => "GGUFTDF_NOT_ZIP",
            Self::NoManifest => "GGUFTDF_NO_MANIFEST",
            Self::UnsupportedProfile(_) => "GGUFTDF_UNSUPPORTED_PROFILE",
            Self::NotGguf => "GGUFTDF_NOT_GGUF",
            Self::UnsupportedGgufVersion(_) => "GGUFTDF_UNSUPPORTED_GGUF_VERSION",
            Self::UnsupportedEndian => "GGUFTDF_UNSUPPORTED_ENDIAN",
            Self::BadAlign(_) => "GGUFTDF_BAD_ALIGN",
            Self::BadMaxSegment(_) => "GGUFTDF_BAD_MAX_SEGMENT",
            Self::BadHeader(_) => "GGUFTDF_BAD_HEADER",
            Self::BadTensor(_) => "GGUFTDF_BAD_TENSOR",
            Self::Overlap => "GGUFTDF_OVERLAP",
            Self::BadIndex(_) | Self::Io(_) => "GGUFTDF_BAD_INDEX",
            Self::SizeMismatch => "GGUFTDF_SIZE_MISMATCH",
            Self::KasDenied(_) => "GGUFTDF_KAS_DENIED",
            Self::TagMismatch => "GGUFTDF_TAG_MISMATCH",
            Self::RootMismatch => "GGUFTDF_ROOT_MISMATCH",
            Self::ReadAtZero => "GGUFTDF_READ_AT_ZERO",
            Self::MtmdUnsupported => "GGUFTDF_MTMD_UNSUPPORTED",
            Self::SiblingRefused => "GGUFTDF_SIBLING_REFUSED",
            Self::PayloadForbidden => "GGUFTDF_PAYLOAD_FORBIDDEN",
            Self::Crypto(_) => "GGUFTDF_CRYPTO",
        }
    }
}

impl From<opentdf::TdfError> for GgufTdfError {
    fn from(err: opentdf::TdfError) -> Self {
        match err {
            opentdf::TdfError::IoError(io) => Self::Io(io),
            opentdf::TdfError::ZipError(zip) => zip.into(),
            other => Self::BadIndex(format!("TDF archive error: {other}")),
        }
    }
}

impl From<zip::result::ZipError> for GgufTdfError {
    fn from(err: zip::result::ZipError) -> Self {
        match err {
            zip::result::ZipError::FileNotFound => Self::NoManifest,
            zip::result::ZipError::Io(io) => Self::Io(io),
            other => Self::BadIndex(format!("zip error: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_reports_its_spec_code() {
        assert_eq!(GgufTdfError::NotZip.code(), "GGUFTDF_NOT_ZIP");
        assert_eq!(GgufTdfError::NoManifest.code(), "GGUFTDF_NO_MANIFEST");
        assert_eq!(
            GgufTdfError::UnsupportedProfile("gguf-tdf/0".into()).code(),
            "GGUFTDF_UNSUPPORTED_PROFILE"
        );
        assert_eq!(GgufTdfError::NotGguf.code(), "GGUFTDF_NOT_GGUF");
        assert_eq!(
            GgufTdfError::UnsupportedGgufVersion(2).code(),
            "GGUFTDF_UNSUPPORTED_GGUF_VERSION"
        );
        assert_eq!(
            GgufTdfError::UnsupportedEndian.code(),
            "GGUFTDF_UNSUPPORTED_ENDIAN"
        );
        assert_eq!(GgufTdfError::BadAlign(24).code(), "GGUFTDF_BAD_ALIGN");
        assert_eq!(
            GgufTdfError::BadMaxSegment(100).code(),
            "GGUFTDF_BAD_MAX_SEGMENT"
        );
        assert_eq!(
            GgufTdfError::BadHeader("x".into()).code(),
            "GGUFTDF_BAD_HEADER"
        );
        assert_eq!(
            GgufTdfError::BadTensor("x".into()).code(),
            "GGUFTDF_BAD_TENSOR"
        );
        assert_eq!(GgufTdfError::Overlap.code(), "GGUFTDF_OVERLAP");
        assert_eq!(
            GgufTdfError::BadIndex("x".into()).code(),
            "GGUFTDF_BAD_INDEX"
        );
        assert_eq!(GgufTdfError::SizeMismatch.code(), "GGUFTDF_SIZE_MISMATCH");
        assert_eq!(
            GgufTdfError::KasDenied("x".into()).code(),
            "GGUFTDF_KAS_DENIED"
        );
        assert_eq!(GgufTdfError::TagMismatch.code(), "GGUFTDF_TAG_MISMATCH");
        assert_eq!(GgufTdfError::RootMismatch.code(), "GGUFTDF_ROOT_MISMATCH");
        assert_eq!(GgufTdfError::ReadAtZero.code(), "GGUFTDF_READ_AT_ZERO");
        assert_eq!(
            GgufTdfError::MtmdUnsupported.code(),
            "GGUFTDF_MTMD_UNSUPPORTED"
        );
        assert_eq!(
            GgufTdfError::SiblingRefused.code(),
            "GGUFTDF_SIBLING_REFUSED"
        );
        assert_eq!(
            GgufTdfError::PayloadForbidden.code(),
            "GGUFTDF_PAYLOAD_FORBIDDEN"
        );
        assert_eq!(GgufTdfError::Crypto("x".into()).code(), "GGUFTDF_CRYPTO");
    }

    #[test]
    fn error_display_includes_the_code() {
        let msg = GgufTdfError::TagMismatch.to_string();
        assert!(msg.contains("GGUFTDF_TAG_MISMATCH"), "got: {msg}");
    }

    #[test]
    fn io_errors_surface_as_bad_index() {
        let err: GgufTdfError =
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "truncated").into();
        assert_eq!(err.code(), "GGUFTDF_BAD_INDEX");
    }
}
