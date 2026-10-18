//! Errors, each with the stable code a requester sees.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImageGenError {
    #[error("invalid {field}: {reason}")]
    InvalidParams { field: &'static str, reason: String },

    #[error("unknown model {name:?}; this host knows {known}")]
    UnknownModel { name: String, known: String },

    #[error("{feature} is not supported: {reason}")]
    UnsupportedFeature {
        feature: &'static str,
        reason: String,
    },

    #[error("this host does not offer {entry}: no weight set fits its memory budget")]
    NotOffered { entry: &'static str },

    #[error(
        "{entry} at {pixels} pixels{guided} does not fit this host; the largest it can render is {largest}"
    )]
    TooLargeForHost {
        entry: &'static str,
        pixels: u64,
        guided: &'static str,
        largest: String,
    },

    #[error("image generation queue is full ({depth} waiting); try another member")]
    Busy { depth: usize },

    #[error("image generation stopped at its deadline")]
    TimedOut,

    #[error("image generation was canceled")]
    Canceled,

    #[error("no swarm member can generate images for {what}")]
    NoCapableMember { what: String },

    #[error("not enough disk space: {needed} bytes needed in {dir}, {available} free")]
    DiskSpace {
        dir: PathBuf,
        needed: u64,
        available: u64,
    },

    #[error("download of {url} failed: {reason}")]
    Download { url: String, reason: String },

    #[error("{name}: expected SHA-256 {expected}, got {actual}")]
    HashMismatch {
        name: String,
        expected: String,
        actual: String,
    },

    #[error("{name}: expected {expected} bytes, got {actual}")]
    SizeMismatch {
        name: String,
        expected: u64,
        actual: u64,
    },

    #[error("runtime install failed: {0}")]
    RuntimeInstall(String),

    #[error("stable-diffusion.cpp exited with {status}: {stderr}")]
    Runtime { status: String, stderr: String },

    #[error("generated output rejected: {0}")]
    Output(String),

    /// The GPU failed during the run; what sd-cli wrote is discarded.
    #[error("the GPU failed while generating, most likely out of memory: {stderr}")]
    GpuFailed { stderr: String },

    #[error("delegated generation failed: {0}")]
    Delegation(String),

    #[error("member {member} reported no progress for {minutes} minutes")]
    Stalled { member: String, minutes: u64 },

    /// An error that reached this holder through a shared generation, with
    /// the code and message it had there.
    #[error("{message}")]
    Shared { code: &'static str, message: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl ImageGenError {
    /// The code requesters match on; message text may change, codes do not.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidParams { .. } | Self::UnknownModel { .. } => "invalid_params",
            Self::UnsupportedFeature { .. } => "unsupported_feature",
            Self::NotOffered { .. } | Self::NoCapableMember { .. } => "no_capable_member",
            Self::TooLargeForHost { .. } => "too_large_for_host",
            Self::Busy { .. } => "busy",
            Self::TimedOut => "timed_out",
            Self::Canceled => "canceled",
            Self::DiskSpace { .. } => "disk_space",
            Self::Download { .. } | Self::HashMismatch { .. } | Self::SizeMismatch { .. } => {
                "fetch_failed"
            }
            Self::RuntimeInstall(_) => "runtime_install_failed",
            Self::Runtime { .. } | Self::Output(_) | Self::GpuFailed { .. } | Self::Io(_) => {
                "generation_failed"
            }
            Self::Delegation(_) => "delegation_failed",
            Self::Stalled { .. } => "step_stalled",
            Self::Shared { code, .. } => code,
        }
    }

    /// Whether another member might succeed where this host refused.
    pub const fn try_elsewhere(&self) -> bool {
        matches!(
            self,
            Self::Busy { .. } | Self::NotOffered { .. } | Self::TooLargeForHost { .. }
        )
    }
}

pub type Result<T> = std::result::Result<T, ImageGenError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable() {
        assert_eq!(ImageGenError::Busy { depth: 4 }.code(), "busy");
        assert_eq!(ImageGenError::TimedOut.code(), "timed_out");
        assert_eq!(
            ImageGenError::NoCapableMember { what: "x".into() }.code(),
            "no_capable_member"
        );
        assert_eq!(
            ImageGenError::UnknownModel {
                name: "x".into(),
                known: "y".into()
            }
            .code(),
            "invalid_params"
        );
    }

    #[arkavo_test_macros::spec("ASSET-002")]
    #[test]
    fn only_capacity_refusals_send_work_elsewhere() {
        assert!(ImageGenError::Busy { depth: 4 }.try_elsewhere());
        assert!(ImageGenError::NotOffered { entry: "e" }.try_elsewhere());
        assert!(!ImageGenError::TimedOut.try_elsewhere());
        assert!(
            !ImageGenError::InvalidParams {
                field: "prompt",
                reason: "empty".into()
            }
            .try_elsewhere()
        );
    }
}
