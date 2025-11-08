# Organization-Wide GitHub Orchestration - Scope of Work

## Executive Summary

**Current State:** The GitHub orchestrator (PR #328) successfully implements single-repository issue polling and webhook processing. It can monitor one repository at a time with robust error handling, state persistence, and rate limiting.

**Business Value Gap:** The enterprise feature is **organization-wide orchestration** - monitoring hundreds of repositories across multiple organizations simultaneously. This is the "big money feature" that differentiates Arkavo from single-repo automation tools.

**Implementation Status:** ❌ Not implemented. Current design is single-repo only.

**Effort Estimate:** 3-4 weeks for core org-wide functionality, 5-6 weeks for production-grade implementation.

---

## Business Requirements

### Primary Use Case
Enable organizations to automatically orchestrate AI agents across **all repositories** without manual configuration per repo.

**Example:** Company "Acme Corp" has 200 repositories across 3 GitHub organizations:
- `acme-backend` (85 repos)
- `acme-frontend` (60 repos)
- `acme-infrastructure` (55 repos)

**Expected behavior:**
```bash
# Deploy organization-wide orchestrator
arkavo orchestrator poll --org acme-backend --org acme-frontend --org acme-infrastructure

# Automatically discovers all 200 repos
# Polls each repo for new issues in parallel
# Routes issues to appropriate AI agents
# Scales as new repos are added
```

### Key Differentiators

1. **Zero per-repo configuration** - Add org token once, monitor all current and future repos
2. **Cross-repository intelligence** - Agents can learn patterns across org-wide codebase
3. **Enterprise scalability** - Handle 500+ repos, 10K+ issues/day
4. **Centralized metrics** - Org-level visibility into AI agent effectiveness

### Success Metrics

- Monitor ≥100 repos concurrently per orchestrator instance
- Poll cycle latency <5 minutes for 500 repos
- Issue processing latency <30 seconds (same as single-repo)
- State recovery time <60 seconds after restart
- API rate limit efficiency >95% (minimal wasted calls)

---

## Technical Requirements

### Functional Requirements

#### FR1: Organization Repository Discovery
- Automatically enumerate all repositories in a GitHub organization
- Support pagination (orgs may have 100s of repos)
- Filter repos by pattern (include/exclude regex)
- Handle archived/disabled repos gracefully
- Refresh repo list periodically (detect new/removed repos)

**API Endpoints Needed:**
- `GET /orgs/{org}/repos` - List organization repositories (paginated)
- `GET /user/orgs` - List user's organizations
- `GET /installation/repositories` - List repositories for GitHub App installation

#### FR2: Multi-Repository Concurrent Polling
- Poll multiple repositories in parallel (not sequential)
- Configurable concurrency limit (e.g., max 10 concurrent polls)
- Independent poll cycles per repo (different intervals if needed)
- Graceful degradation if one repo fails (don't block others)

**Architecture:**
```
OrganizationOrchestrator
  ├─ OrgPoller (acme-backend)
  │  ├─ RepoPoller (acme-backend/api-gateway) [tokio task]
  │  ├─ RepoPoller (acme-backend/auth-service) [tokio task]
  │  └─ RepoPoller (acme-backend/payment-service) [tokio task]
  ├─ OrgPoller (acme-frontend)
  │  ├─ RepoPoller (acme-frontend/web-app) [tokio task]
  │  └─ RepoPoller (acme-frontend/mobile-app) [tokio task]
  └─ MetricsAggregator (centralized)
```

#### FR3: Persistent State Management
- Track processed issues across all repos in durable storage
- Prevent duplicate processing after restart
- Cleanup old state (retention policy: 7 days)
- Support state queries (e.g., "show issues processed in last 24h")

**Current:** JSON files per repo (`~/.arkavo/orchestrator-poll-owner-repo.json`)
**Required:** Centralized SQLite database (`~/.arkavo/orchestrator-org-state.db`)

**Schema:**
```sql
CREATE TABLE repo_state (
    org TEXT NOT NULL,
    repo_name TEXT NOT NULL,
    last_poll_at DATETIME NOT NULL,
    issues_processed INTEGER DEFAULT 0,
    last_error TEXT,
    error_count INTEGER DEFAULT 0,
    status TEXT CHECK(status IN ('active', 'paused', 'failed')),
    PRIMARY KEY (org, repo_name)
);

CREATE TABLE processed_issues (
    org TEXT NOT NULL,
    repo_name TEXT NOT NULL,
    issue_number INTEGER NOT NULL,
    task_id TEXT NOT NULL,
    processed_at DATETIME NOT NULL,
    PRIMARY KEY (org, repo_name, issue_number)
);

CREATE INDEX idx_processed_at ON processed_issues(processed_at);
```

#### FR4: Error Isolation & Circuit Breaker
- Repository-level error tracking (don't let one bad repo crash orchestrator)
- Circuit breaker pattern: After N failures, pause repo polling for X minutes
- Dead letter queue for failed issues (retry later)
- Health status per repo: `healthy`, `degraded`, `failed`

**Configuration:**
```rust
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,      // Default: 5
    pub timeout_duration: Duration,  // Default: 15 minutes
    pub half_open_retries: u32,      // Default: 1
}
```

#### FR5: Organization-Level Metrics
- Aggregated statistics across all repos in org
- Per-repo drill-down (issues processed, errors, latency)
- API rate limit usage by org
- Export to Prometheus/OpenTelemetry

**Metrics Structure:**
```rust
pub struct OrgMetrics {
    pub org_name: String,
    pub repos_total: usize,
    pub repos_active: usize,
    pub repos_paused: usize,
    pub repos_failed: usize,
    pub issues_processed_total: u64,
    pub issues_failed_total: u64,
    pub avg_processing_time_ms: f64,
    pub api_calls_last_hour: u32,
    pub rate_limit_remaining: u32,
    pub last_updated_at: DateTime<Utc>,
}
```

#### FR6: Dynamic Configuration
- Add/remove organizations at runtime (no restart)
- Update polling intervals per org/repo
- Enable/disable specific repos
- Configuration via CLI or API

**CLI Commands:**
```bash
# Add organization
arkavo orchestrator config add-org acme-backend

# Remove organization
arkavo orchestrator config remove-org acme-backend

# Pause specific repo
arkavo orchestrator config pause-repo acme-backend/legacy-monolith

# Show current config
arkavo orchestrator config show
```

---

### Non-Functional Requirements

#### NFR1: Performance
- Poll 100 repos in <5 minutes (avg 3 sec/repo)
- Support 500+ repos per orchestrator instance
- Handle 10K issues/day throughput
- Memory usage <2GB for 500 repos

#### NFR2: Scalability
- Horizontal scaling: Run multiple orchestrator instances for different orgs
- No shared state conflicts (each instance independent)
- Support 10+ GitHub organizations per instance
- Graceful handling of rate limits across parallel polls

#### NFR3: Reliability
- Automatic recovery from crashes (state persisted)
- No duplicate issue processing after restart
- Graceful degradation under GitHub API failures
- Health check endpoint for monitoring

#### NFR4: Security
- Support GitHub App authentication (preferred for orgs)
- Support fine-grained PAT for testing
- Secure storage of tokens (not in config files)
- Audit log of all issue processing actions

#### NFR5: Observability
- Structured logging (JSON format)
- Metrics export (Prometheus format)
- Distributed tracing (OpenTelemetry)
- Dashboard for org-wide status

---

## Architecture Design

### Component Breakdown

#### 1. Organization Discovery Service
**New crate:** `crates/arkavo-org-discovery/`

**Purpose:** Enumerate and cache organization repositories

**Key Types:**
```rust
pub struct OrgDiscovery {
    github_client: Arc<GitHubApp>,
    cache: Arc<RwLock<HashMap<String, CachedRepoList>>>,
    cache_ttl: Duration,
}

pub struct RepoInfo {
    pub full_name: String,
    pub owner: String,
    pub name: String,
    pub is_archived: bool,
    pub default_branch: String,
    pub language: Option<String>,
    pub size_kb: u64,
}

impl OrgDiscovery {
    pub async fn discover_repos(&self, org: &str) -> Result<Vec<RepoInfo>>;
    pub async fn refresh_cache(&self, org: &str) -> Result<()>;
    pub async fn filter_repos(&self, pattern: &str) -> Result<Vec<RepoInfo>>;
}
```

**Files:**
- `src/discovery.rs` - Core discovery logic
- `src/cache.rs` - Repository list caching
- `src/filters.rs` - Regex filtering
- `src/error.rs` - Error types

#### 2. Multi-Repo Poller
**Modified crate:** `crates/arkavo-orchestrator/`

**New file:** `src/org_poller.rs`

**Purpose:** Coordinate polling across multiple repos

**Key Types:**
```rust
pub struct OrgPoller {
    org_name: String,
    repos: Vec<String>,
    orchestrator: Arc<Orchestrator>,
    discovery: Arc<OrgDiscovery>,
    state_store: Arc<StateStore>,
    config: OrgPollerConfig,
    active_tasks: Arc<RwLock<HashMap<String, JoinHandle<()>>>>,
}

pub struct OrgPollerConfig {
    pub poll_interval: Duration,
    pub max_concurrent_repos: usize,
    pub discovery_interval: Duration,
    pub circuit_breaker: CircuitBreakerConfig,
}

impl OrgPoller {
    pub async fn start(&self) -> Result<()>;
    pub async fn stop(&self) -> Result<()>;
    pub async fn add_repo(&self, repo: String) -> Result<()>;
    pub async fn remove_repo(&self, repo: String) -> Result<()>;

    async fn poll_repo(&self, repo: String) -> Result<()>;
    async fn discover_new_repos(&self) -> Result<()>;
}
```

#### 3. Persistent State Store
**New file:** `crates/arkavo-orchestrator/src/state_store.rs`

**Purpose:** Centralized state management using SQLite

**Key Types:**
```rust
pub struct StateStore {
    db: Arc<SqlitePool>,
}

impl StateStore {
    pub async fn new(path: &Path) -> Result<Self>;

    // Repo state
    pub async fn update_repo_state(&self, state: RepoState) -> Result<()>;
    pub async fn get_repo_state(&self, org: &str, repo: &str) -> Result<Option<RepoState>>;
    pub async fn list_active_repos(&self, org: &str) -> Result<Vec<RepoState>>;

    // Issue tracking
    pub async fn mark_issue_processed(&self, org: &str, repo: &str, issue: u64, task_id: Uuid) -> Result<()>;
    pub async fn is_issue_processed(&self, org: &str, repo: &str, issue: u64) -> Result<bool>;
    pub async fn cleanup_old_issues(&self, before: DateTime<Utc>) -> Result<u64>;

    // Metrics
    pub async fn record_poll_metrics(&self, metrics: PollMetrics) -> Result<()>;
    pub async fn get_org_metrics(&self, org: &str) -> Result<OrgMetrics>;
}
```

**Schema:** See FR3 above

#### 4. Circuit Breaker
**New file:** `crates/arkavo-orchestrator/src/circuit_breaker.rs`

**Purpose:** Prevent cascading failures from bad repos

**Key Types:**
```rust
pub struct CircuitBreaker {
    state: Arc<RwLock<CircuitBreakerState>>,
    config: CircuitBreakerConfig,
}

pub enum CircuitBreakerState {
    Closed,
    Open { opened_at: DateTime<Utc> },
    HalfOpen { retry_count: u32 },
}

impl CircuitBreaker {
    pub async fn call<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T>;

    async fn record_success(&self);
    async fn record_failure(&self);
}
```

#### 5. Metrics Aggregator
**Modified file:** `crates/arkavo-orchestrator/src/orchestrator.rs`

**New Trait:**
```rust
pub trait MetricsProvider {
    fn get_org_metrics(&self, org: &str) -> OrgMetrics;
    fn get_repo_metrics(&self, org: &str, repo: &str) -> RepoMetrics;
    fn export_prometheus(&self) -> String;
}
```

#### 6. CLI Extensions
**Modified file:** `crates/arkavo-cli/src/commands/orchestrator/commands.rs`

**New Commands:**
```rust
pub enum OrchestratorSubcommand {
    // Existing
    Start { ... },
    Poll { repo: String, ... },
    Process { ... },

    // New for org-wide
    PollOrg {
        #[arg(long, value_name = "ORG", required = true)]
        org: Vec<String>,  // Multiple orgs

        #[arg(long, short = 'i', default_value = "300")]
        interval: u64,

        #[arg(long)]
        once: bool,

        #[arg(long)]
        token: Option<String>,

        #[arg(long)]
        labels: Option<String>,

        #[arg(long)]
        repo_include: Option<String>,  // Regex pattern

        #[arg(long)]
        repo_exclude: Option<String>,

        #[arg(long, default_value = "10")]
        max_concurrent: usize,
    },

    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },

    Metrics {
        #[arg(long)]
        org: Option<String>,

        #[arg(long)]
        repo: Option<String>,

        #[arg(long)]
        format: Option<MetricsFormat>,  // json | prometheus | table
    },
}

pub enum ConfigCommand {
    Show,
    AddOrg { org: String },
    RemoveOrg { org: String },
    PauseRepo { repo: String },
    ResumeRepo { repo: String },
}
```

---

## Implementation Plan

### Phase 1: Foundation (Week 1)
**Goal:** Basic org discovery and state management

**Tasks:**
1. Create `arkavo-org-discovery` crate
   - GitHub org API wrapper
   - Repository enumeration with pagination
   - Basic caching (in-memory)

2. Create centralized state store
   - SQLite schema design
   - Migration from JSON to SQLite
   - Basic CRUD operations

3. Unit tests for discovery and state store

**Deliverables:**
- `crates/arkavo-org-discovery/` (300 lines)
- `crates/arkavo-orchestrator/src/state_store.rs` (400 lines)
- Migration script for existing poll state
- Tests with ≥85% coverage

**Acceptance Criteria:**
- Can enumerate all repos in `arkavo-org`
- State persists across process restarts
- No data loss during migration

---

### Phase 2: Multi-Repo Polling (Week 2)
**Goal:** Poll multiple repos concurrently

**Tasks:**
1. Implement `OrgPoller` coordinator
   - Spawn tokio task per repo
   - Bounded concurrency (Semaphore)
   - Error isolation per repo

2. Extend CLI with `poll-org` command
   - Parse org list
   - Pass config to OrgPoller

3. Integration tests with mock GitHub API

**Deliverables:**
- `crates/arkavo-orchestrator/src/org_poller.rs` (350 lines)
- Updated `commands.rs` (200 lines)
- Integration tests (300 lines)

**Acceptance Criteria:**
- Poll 10 repos concurrently
- One repo failure doesn't block others
- Logs show parallel execution

---

### Phase 3: Circuit Breaker & Error Handling (Week 3)
**Goal:** Production-grade error resilience

**Tasks:**
1. Implement circuit breaker pattern
   - Track failure counts per repo
   - Automatic pause/resume
   - Configurable thresholds

2. Add metrics aggregation
   - Per-repo metrics
   - Per-org rollup
   - Prometheus export

3. Dead letter queue for failed issues

**Deliverables:**
- `crates/arkavo-orchestrator/src/circuit_breaker.rs` (250 lines)
- `crates/arkavo-orchestrator/src/metrics_aggregator.rs` (300 lines)
- Metrics documentation

**Acceptance Criteria:**
- Repo paused after 5 failures
- Metrics endpoint returns valid Prometheus format
- Failed issues can be retried manually

---

### Phase 4: Dynamic Configuration & Polish (Week 4)
**Goal:** Runtime management and observability

**Tasks:**
1. Configuration management
   - Add/remove orgs without restart
   - Pause/resume individual repos
   - Config persistence

2. Metrics dashboard
   - CLI table output
   - JSON export
   - Grafana dashboard template

3. Documentation and examples

**Deliverables:**
- `crates/arkavo-orchestrator/src/config_manager.rs` (200 lines)
- Updated CLI commands (150 lines)
- User documentation
- Grafana dashboard JSON

**Acceptance Criteria:**
- Add org via CLI, new repos polled within 5 minutes
- Dashboard shows real-time status
- Documentation covers all org-wide features

---

### Phase 5: Testing & Optimization (Week 5-6, Optional)
**Goal:** Production readiness

**Tasks:**
1. Load testing (simulate 500 repos)
2. Performance optimization
3. Memory leak detection
4. Chaos engineering (random API failures)
5. Security audit (token handling, rate limits)

**Deliverables:**
- Load test results
- Performance tuning guide
- Security review report
- Regression test suite

**Acceptance Criteria:**
- Handle 500 repos with <2GB memory
- 99.9% uptime over 7 day test
- No critical security findings

---

## File Changes Summary

### New Files (8 files, ~2400 lines)
```
crates/arkavo-org-discovery/
  src/lib.rs                        (50 lines)
  src/discovery.rs                  (300 lines)
  src/cache.rs                      (150 lines)
  src/filters.rs                    (100 lines)
  src/error.rs                      (50 lines)
  Cargo.toml                        (20 lines)

crates/arkavo-orchestrator/src/
  org_poller.rs                     (350 lines)
  state_store.rs                    (400 lines)
  circuit_breaker.rs                (250 lines)
  metrics_aggregator.rs             (300 lines)
  config_manager.rs                 (200 lines)

docs/
  orchestrator-org-wide-guide.md    (500 lines - user guide)
  orchestrator-org-wide-api.md      (300 lines - API reference)
```

### Modified Files (5 files, ~500 lines changed)
```
crates/arkavo-cli/src/commands/orchestrator/
  commands.rs                       (+200 lines - new commands)
  polling.rs                        (+100 lines - org support)

crates/arkavo-orchestrator/src/
  orchestrator.rs                   (+100 lines - metrics trait)
  config.rs                         (+50 lines - org config)
  lib.rs                            (+50 lines - exports)
```

### Test Files (4 files, ~1200 lines)
```
crates/arkavo-org-discovery/tests/
  integration_test.rs               (300 lines)

crates/arkavo-orchestrator/tests/
  org_poller_test.rs                (400 lines)
  state_store_test.rs               (300 lines)
  circuit_breaker_test.rs           (200 lines)
```

**Total:** ~4100 lines of new code

---

## Risk Assessment

### High Risk
1. **GitHub API rate limits** - 5000 requests/hour for authenticated users
   - Mitigation: Cache repo lists, stagger polls, use conditional requests

2. **State synchronization** - Multiple orchestrator instances
   - Mitigation: SQLite locks, leader election, or separate DBs per instance

3. **Memory usage** - 500 repos × state × tasks
   - Mitigation: Lazy loading, state cleanup, task limits

### Medium Risk
1. **Backward compatibility** - Existing single-repo users
   - Mitigation: Keep `poll` command, add new `poll-org` command

2. **Testing complexity** - Hard to test 500-repo scenarios
   - Mitigation: Mock GitHub API, synthetic data generation

### Low Risk
1. **Security** - Token exposure in logs
   - Mitigation: Sanitize logs, use secret manager

2. **Migration** - JSON → SQLite state
   - Mitigation: Write migration tool, test on real data

---

## Success Criteria

### Functional Success
- ✅ Poll 100+ repos concurrently
- ✅ Auto-discover new repos within 5 minutes
- ✅ Zero duplicate issue processing
- ✅ Survive 50% API failure rate
- ✅ State recovery in <60 seconds

### Code Quality Success
- ✅ All files <400 lines
- ✅ Test coverage ≥85%
- ✅ Zero clippy warnings
- ✅ Documentation complete
- ✅ Regression tests for bugs

### Business Success
- ✅ Deploy to arkavo-org (200+ repos)
- ✅ Process 1000+ issues/week
- ✅ Zero production incidents
- ✅ Positive user feedback
- ✅ Onboarding time <10 minutes

---

## Dependencies & Prerequisites

### Required Skills
- Rust async programming (tokio)
- GitHub API (REST v3)
- SQLite/SQL
- Concurrent systems design
- Error handling patterns

### External Dependencies
```toml
# New dependencies
sqlx = { version = "0.7", features = ["sqlite", "runtime-tokio"] }
regex = "1.10"
governor = "0.6"  # Rate limiting
prometheus = "0.13"  # Metrics

# Already in use
octocrab = "0.38"  # GitHub API
tokio = { version = "1.36", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
```

### Infrastructure
- SQLite 3.35+ (embedded)
- GitHub App (optional, for webhook mode)
- GitHub PAT with `repo` + `read:org` scopes

---

## Open Questions

1. **Multi-tenancy:** Should one orchestrator instance handle multiple customers' orgs?
   - Proposal: No. One instance = one customer = multiple orgs.

2. **Webhook vs Polling:** Recommend one mode for org-wide?
   - Proposal: Start with polling (simpler), add org-wide webhook in Phase 6.

3. **State storage:** SQLite vs Postgres vs Redis?
   - Proposal: SQLite for now (single instance), Postgres for multi-instance later.

4. **API design:** REST API for config changes or CLI only?
   - Proposal: CLI only for MVP, REST API in future.

5. **Pricing model:** How to charge for org-wide?
   - Proposal: Per-org tier (e.g., $99/org/month for unlimited repos).

---

## Next Steps

1. **Get stakeholder approval** on scope and timeline
2. **Create GitHub issue** for org-wide orchestration epic
3. **Break down into sub-issues** (one per phase)
4. **Set up project board** for tracking
5. **Assign engineer** to Phase 1
6. **Schedule design review** (before coding starts)

---

## Appendix: Example Usage

### Scenario: Acme Corp Setup

**Step 1: Initial setup**
```bash
# Create GitHub PAT with org:read + repo access
# https://github.com/settings/tokens/new

export GITHUB_TOKEN=ghp_xxxxxxxxxxxx

# Start org-wide orchestrator
arkavo orchestrator poll-org \
  --org acme-backend \
  --org acme-frontend \
  --org acme-infrastructure \
  --interval 300 \
  --max-concurrent 20 \
  --repo-exclude ".*-archive" \
  --labels "ai-ready,automation"
```

**Output:**
```
[INFO] Starting organization-wide orchestrator
[INFO] Discovering repos for acme-backend... found 85 repos
[INFO] Discovering repos for acme-frontend... found 60 repos
[INFO] Discovering repos for acme-infrastructure... found 55 repos
[INFO] Filtered to 180 repos (20 excluded, 0 archived)
[INFO] Polling 180 repos every 300 seconds with max 20 concurrent
[INFO] State stored in /Users/acme/.arkavo/orchestrator-org-state.db
[INFO] Orchestrator ready. Press Ctrl+C to stop.

[INFO] Poll cycle 1/∞
[INFO] Processing acme-backend/api-gateway... 3 new issues
[INFO] Processing acme-backend/auth-service... 0 new issues
[INFO] Processing acme-frontend/web-app... 5 new issues
...
[INFO] Poll cycle complete. Processed 47 issues across 180 repos in 78s
[INFO] Next poll in 222 seconds
```

**Step 2: Monitor metrics**
```bash
arkavo orchestrator metrics --org acme-backend --format table
```

**Output:**
```
┌─────────────────────────────┬─────────┬──────────┬────────┬─────────────────────┐
│ Repository                  │ Status  │ Issues   │ Errors │ Last Poll           │
├─────────────────────────────┼─────────┼──────────┼────────┼─────────────────────┤
│ acme-backend/api-gateway    │ healthy │ 127      │ 0      │ 2 minutes ago       │
│ acme-backend/auth-service   │ healthy │ 43       │ 0      │ 3 minutes ago       │
│ acme-backend/legacy-monolith│ paused  │ 0        │ 12     │ 1 hour ago          │
│ ...                         │         │          │        │                     │
└─────────────────────────────┴─────────┴──────────┴────────┴─────────────────────┘

Organization Summary:
  Total repos: 85
  Active: 82
  Paused: 3 (circuit breaker)
  Failed: 0
  Issues processed (24h): 1,247
  API calls (1h): 342 / 5000
```

**Step 3: Manage failing repo**
```bash
# Check why legacy-monolith is paused
arkavo orchestrator config show-repo acme-backend/legacy-monolith

# Output:
# Status: paused (circuit breaker open)
# Last error: GitHub API 404: Repository not found
# Failure count: 12
# Retry at: 2025-11-07 20:45:00 UTC

# Remove from watch list
arkavo orchestrator config remove-repo acme-backend/legacy-monolith
```

---

**Document Version:** 1.0
**Author:** Claude (Arkavo AI Assistant)
**Date:** 2025-11-07
**Status:** Draft - Awaiting Approval
