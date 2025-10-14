#![allow(clippy::disallowed_methods)]
#![allow(clippy::future_not_send)]
#![allow(dead_code)]
#![allow(clippy::format_push_string)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::unnecessary_debug_formatting)]
#![allow(clippy::lines_filter_map_ok)]
#![allow(clippy::manual_strip)]
#![allow(clippy::needless_continue)]
#![allow(unused_imports)]
#![allow(clippy::zombie_processes)]
#![allow(clippy::unreadable_literal)]
#![allow(clippy::ignore_without_reason)]
#![allow(clippy::unnecessary_unwrap)]
#![allow(unreachable_pub)]

pub mod assertions;
pub mod client;
pub mod fixtures;
pub mod process;

pub use assertions::{WaitError, wait_for_condition, wait_for_ready};
pub use client::{TestHttpClient, TestWebSocketClient};
pub use fixtures::{AgentConfig, TestEnvironment};
pub use process::spawn_component;
