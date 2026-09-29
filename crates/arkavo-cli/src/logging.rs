//! Diagnostic logging for the command line.
//!
//! Standard output belongs to the command's result: `arkavo chat --prompt`
//! is piped and captured, and a log line in that stream becomes part of the
//! model's answer. `tracing_subscriber` writes to stdout unless told
//! otherwise, so the writer is named here.

use tracing_subscriber::{EnvFilter, fmt};

/// Install the process-wide subscriber. Logs go to stderr, at error level
/// unless `RUST_LOG` asks for more.
///
/// A subscriber installed earlier by a host embedding the CLI is left in
/// place; replacing it is not possible and failing over it would take the
/// command down for the sake of its diagnostics.
pub fn init() {
    let _ = fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_env_filter(
            // Default to error-only for clean CLI output; use RUST_LOG for more
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("error")),
        )
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .try_init();
}
