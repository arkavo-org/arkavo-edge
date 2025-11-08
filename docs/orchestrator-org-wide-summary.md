# Organization-Wide Orchestration - Executive Summary

## The Gap

**Current:** GitHub orchestrator polls ONE repository at a time
**Required:** Organization-wide orchestration across 100+ repositories simultaneously
**Business Impact:** This is the enterprise "big money feature"

## What's Missing

The implementation from PR #328 is single-repo only:

```bash
# Current - works
arkavo orchestrator poll --repo arkavo-org/arkavo-edge

# Required - doesn't exist
arkavo orchestrator poll-org --org arkavo-org
# Should auto-discover and monitor ALL 200+ repos
```

## Core Problems

1. **No repository discovery** - Can't enumerate org repos via GitHub API
2. **Sequential polling** - One repo blocks others (not concurrent)
3. **No persistent state** - Uses JSON files, doesn't scale to 100+ repos
4. **No error isolation** - One bad repo crashes entire orchestrator
5. **No org-level metrics** - Can't see aggregate stats across repos

## Solution Architecture

### New Components Needed

1. **Organization Discovery Service** (`arkavo-org-discovery` crate)
   - Enumerate repos via GitHub API
   - Cache repo lists
   - Filter by pattern (include/exclude)

2. **Multi-Repo Concurrent Poller**
   - Spawn tokio task per repo
   - Bounded concurrency (max 10-20 concurrent)
   - Independent poll cycles

3. **Persistent State Store**
   - SQLite database (not JSON files)
   - Track processed issues across all repos
   - Survive restarts

4. **Circuit Breaker**
   - Per-repo error tracking
   - Auto-pause failing repos
   - Prevent cascading failures

5. **Metrics Aggregator**
   - Org-level statistics
   - Per-repo drill-down
   - Prometheus export

### CLI Changes

**New command:**
```bash
arkavo orchestrator poll-org \
  --org acme-backend \
  --org acme-frontend \
  --interval 300 \
  --max-concurrent 20 \
  --repo-exclude ".*-archive"
```

**New config commands:**
```bash
arkavo orchestrator config add-org <org>
arkavo orchestrator config remove-org <org>
arkavo orchestrator config pause-repo <repo>
arkavo orchestrator metrics --org <org>
```

## Implementation Effort

**Timeline:** 4-6 weeks

### Phase 1 (Week 1): Foundation
- GitHub org repo discovery API
- SQLite state store
- Migration from JSON

### Phase 2 (Week 2): Concurrent Polling
- Multi-repo poller coordinator
- Bounded concurrency
- Error isolation

### Phase 3 (Week 3): Reliability
- Circuit breaker pattern
- Metrics aggregation
- Dead letter queue

### Phase 4 (Week 4): Operations
- Dynamic config (add/remove orgs)
- Metrics dashboard
- Documentation

### Phase 5-6 (Optional): Production Hardening
- Load testing (500 repos)
- Performance optimization
- Security audit

## Code Impact

**New files:** ~2400 lines
- `crates/arkavo-org-discovery/` (650 lines)
- `crates/arkavo-orchestrator/src/org_poller.rs` (350 lines)
- `crates/arkavo-orchestrator/src/state_store.rs` (400 lines)
- `crates/arkavo-orchestrator/src/circuit_breaker.rs` (250 lines)
- `crates/arkavo-orchestrator/src/metrics_aggregator.rs` (300 lines)
- Tests (1200 lines)
- Docs (800 lines)

**Modified files:** ~500 lines
- CLI commands (200 lines)
- Orchestrator core (300 lines)

**Total:** ~4100 lines

## Success Metrics

- ✅ Poll 100+ repos concurrently
- ✅ Poll cycle <5 minutes for 500 repos
- ✅ Zero duplicate issue processing
- ✅ State recovery <60 seconds
- ✅ API rate limit efficiency >95%
- ✅ Test coverage ≥85%
- ✅ All files <400 lines

## Business Value

### Before (Single Repo)
```
Manual setup per repo:
  200 repos × 5 minutes = 16.7 hours setup
  Ongoing management: error-prone, no visibility
```

### After (Org-Wide)
```
One-time setup per org:
  1 command × 2 minutes = 2 minutes setup
  Auto-discovery: zero maintenance
  Centralized metrics: full visibility
```

### Pricing Potential
- **Tier 1:** Free (single repo, current feature)
- **Tier 2:** $99/month per organization (<50 repos)
- **Tier 3:** $299/month per organization (50-200 repos)
- **Enterprise:** Custom pricing (200+ repos, SLA)

## Risk Mitigation

**High Risk:**
- GitHub API rate limits → Cache aggressively, stagger polls
- State synchronization → SQLite locks, single instance for MVP
- Memory usage → Lazy loading, cleanup policies

**Medium Risk:**
- Backward compatibility → Keep existing `poll` command
- Testing complexity → Mock GitHub API, synthetic data

## Next Steps

1. Review and approve scope document
2. Create GitHub epic issue
3. Break down into sub-issues (one per phase)
4. Assign to engineer
5. Start Phase 1 (Week 1)

## References

- Full scope: `docs/orchestrator-org-wide-scope.md`
- Current implementation: PR #328
- Testing guide: `docs/testing-orchestrator.md`
