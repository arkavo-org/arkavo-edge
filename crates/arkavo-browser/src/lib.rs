mod browser;
mod error;
pub mod live_inject;

pub use browser::BrowserTool;
pub use error::{BrowserError, Result};
pub use live_inject::{LiveInjector, create_injector_for_browser};
