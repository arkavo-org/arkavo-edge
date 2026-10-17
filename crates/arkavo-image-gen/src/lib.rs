//! Local image generation for Arkavo Edge agents.
//!
//! A host that qualifies (macOS on Apple Silicon, with the memory for at
//! least one catalog entry) runs Qwen-Image models through a pinned
//! stable-diffusion.cpp build on Metal. Nothing here calls a remote
//! inference service. Behavior: `specs/arkavo-edge/image-gen.spec.yaml`.

pub mod args;
pub mod catalog;
pub mod delivery;
pub mod descriptor;
pub mod error;
pub mod fetch;
pub mod generator;
pub mod host;
pub mod hosting;
pub mod job;
mod lock;
pub mod output;
pub mod plan;
pub mod progress;
pub mod request;
pub mod run;
pub mod runtime;
pub mod store;
pub mod tool;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use catalog::{Entry, RUNTIME, SetLabel, WeightSet};
pub use delivery::{AssetRequest, PNG, safe_name, verify_fetched};
pub use descriptor::{
    ASSET_MANIFEST_SCHEMA, ASSET_REQUEST_SCHEMA, CAPABILITY, Descriptor, Need, rank,
};
pub use error::{ImageGenError, Result};
pub use generator::Generator;
pub use host::{Host, Platform};
pub use hosting::HostGenerator;
pub use job::{AssetJob, AssetSink, Outbox};
pub use output::{AssetEntry, Generated, Manifest, Producer, assets_dir};
pub use plan::{Offer, offers};
pub use progress::Progress;
pub use request::{ImageRequest, Job};
pub use store::Store;
pub use tool::{Delegate, GenerateImageTool, register_tools};
