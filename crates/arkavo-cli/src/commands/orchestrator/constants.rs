pub(super) const MAX_IP_ENTRIES: usize = 10_000;
pub(super) const IP_ENTRY_TTL_SECONDS: u64 = 3600;
pub(super) const MAX_PROCESSED_ISSUES: usize = 1000;
pub(super) const MAX_RATE_LIMIT_RETRIES: usize = 3;
pub(super) const GITHUB_API_TIMEOUT_SECS: u64 = 30;
pub(super) const USER_AGENT: &str = concat!("arkavo-edge/", env!("CARGO_PKG_VERSION"));
