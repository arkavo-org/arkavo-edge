# SwarmKit PR-review — WS-B (GitHub PR capability) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `github_pr_watch` MCP tool to `arkavo-mcp-tools` that lists a repo's open PRs and returns each one's identity + `head_sha` + `updated_at`, optionally filtered to those updated after a caller-supplied `since` cursor — so the `dispatcher` role can discover new/changed PRs.

**Architecture:** Self-contained in `arkavo-mcp-tools`, mirroring the existing reqwest + `GITHUB_TOKEN` pattern in `github_api.rs` (no new crate dependency, no cycle). The tool is **stateless**: it returns `head_sha`/`updated_at` and takes an optional `since`; dedup/cursor management lives with the caller (the swarm), keeping the tool simple and composable. The PR-selection logic is a pure function so it is unit-testable without network access.

**Tech Stack:** Rust, `reqwest` (already a dep), `serde`/`serde_json`, `async-trait`, `arkavo-mcp-tools` `Tool` trait.

## Global Constraints

- No `--release` builds; use debug.
- No clippy warnings: `cargo clippy -p arkavo-mcp-tools -- -D warnings`. `#[allow(dead_code)]` forbidden.
- Implementation additions to `github_api.rs` keep the file's non-test code under 400 lines; if it would exceed, put the new tool in a new `github_pr_watch.rs` module instead and report it.
- No new crate dependencies (reqwest/serde_json already present). If `Cargo.toml` changes, commit `Cargo.lock`.
- No Conventional Commits prefixes. Use the exact commit message below incl. its `Co-Authored-By` / `Claude-Session` trailer.
- Tests must not hit the network — test the pure selection function, not live GitHub.
- The tool is stateless (no persisted cursor inside the tool); do not add `MemoryStorage`/`StateStore` threading for this task.

## File Structure

- `crates/arkavo-mcp-tools/src/github_api.rs` — extend `GhPullRequest`; add `GhHead`, `PrSummary`, the pure `select_updated_prs`, and `GitHubPrWatchTool`. (If this pushes the file's non-test code over 400 lines, create `crates/arkavo-mcp-tools/src/github_pr_watch.rs` and `pub mod` it in `lib.rs` instead.)
- `crates/arkavo-mcp-tools/src/registry.rs` — register `github_pr_watch` in the GitHub section of `register_all`.
- `crates/arkavo-mcp-tools/src/lib.rs` — only if a new module file is created.

---

### Task 1: `github_pr_watch` tool + pure PR-selection function

**Files:**
- Modify: `crates/arkavo-mcp-tools/src/github_api.rs`
- Modify: `crates/arkavo-mcp-tools/src/registry.rs`
- Test: inline `#[cfg(test)]` in `github_api.rs`

**Interfaces:**
- Consumes (existing in `github_api.rs`): `get_github_client()`, `github_request(gh, request, action)`, `GITHUB_API_BASE`, the `Tool` trait, `ToolSchema`, `ToolError`, `crate::Result`.
- Produces: tool `github_pr_watch`; pure `fn select_updated_prs(prs: Vec<GhPullRequest>, since: Option<&str>) -> Vec<PrSummary>`; `struct PrSummary { number, title, url, author, head_sha, updated_at }`.

- [ ] **Step 1: Read the existing pattern**

Read `crates/arkavo-mcp-tools/src/github_api.rs` around the client helpers (`get_github_client`, `github_request`, `GITHUB_API_BASE`) and `GitHubPrListTool` (~lines 26-31 for `GhPullRequest`, ~287-371 for the tool) so the new code matches the established request/parse/error idiom exactly.

- [ ] **Step 2: Write the failing test for `select_updated_prs`**

Add to the `#[cfg(test)]` module in `github_api.rs` (create one if absent):

```rust
#[cfg(test)]
mod pr_watch_tests {
    use super::*;

    fn pr(number: u64, sha: &str, updated: &str) -> GhPullRequest {
        GhPullRequest {
            number,
            title: Some(format!("PR {number}")),
            html_url: Some(format!("https://example/pr/{number}")),
            state: Some("open".into()),
            user: Some(GhUser { login: Some("alice".into()) }),
            head: Some(GhHead { sha: sha.into() }),
            updated_at: Some(updated.into()),
        }
    }

    #[test]
    fn no_since_returns_all() {
        let prs = vec![pr(1, "aaa", "2026-06-18T10:00:00Z"), pr(2, "bbb", "2026-06-18T12:00:00Z")];
        let out = select_updated_prs(prs, None);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].head_sha, "aaa");
    }

    #[test]
    fn since_excludes_unchanged_and_includes_newer() {
        let prs = vec![
            pr(1, "aaa", "2026-06-18T10:00:00Z"), // unchanged: == since
            pr(2, "bbb", "2026-06-18T13:00:00Z"), // changed: newer than since
        ];
        let out = select_updated_prs(prs, Some("2026-06-18T10:00:00Z"));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].number, 2);
        assert_eq!(out[0].head_sha, "bbb");
    }

    #[test]
    fn missing_updated_at_is_excluded_when_since_set() {
        let mut p = pr(3, "ccc", "");
        p.updated_at = None;
        let out = select_updated_prs(vec![p], Some("2026-06-18T10:00:00Z"));
        assert!(out.is_empty());
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p arkavo-mcp-tools pr_watch_tests`
Expected: FAIL to compile — `GhHead`, `PrSummary`, `select_updated_prs`, and the new `GhPullRequest` fields don't exist yet.

- [ ] **Step 4: Extend `GhPullRequest` and add the data types + pure function**

In `github_api.rs`, extend the existing `GhPullRequest` struct with two optional fields (keep existing fields/derives):

```rust
    head: Option<GhHead>,
    updated_at: Option<String>,
```

Add the supporting types and the pure selection function:

```rust
#[derive(Debug, Clone, serde::Deserialize)]
struct GhHead {
    sha: String,
}

/// One PR as surfaced to the swarm: identity + head SHA + last-update time.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PrSummary {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    pub head_sha: String,
    pub updated_at: String,
}

/// Pure: map open PRs to summaries, keeping only those updated strictly after
/// `since` (lexicographic compare is correct for ISO-8601 UTC `Z` timestamps).
/// PRs missing a head SHA or `updated_at` are dropped (can't be tracked).
fn select_updated_prs(prs: Vec<GhPullRequest>, since: Option<&str>) -> Vec<PrSummary> {
    prs.into_iter()
        .filter_map(|p| {
            let head_sha = p.head.as_ref().map(|h| h.sha.clone())?;
            let updated_at = p.updated_at.clone()?;
            if let Some(since) = since {
                if updated_at.as_str() <= since {
                    return None;
                }
            }
            Some(PrSummary {
                number: p.number,
                title: p.title.unwrap_or_default(),
                url: p.html_url.unwrap_or_default(),
                author: p.user.and_then(|u| u.login).unwrap_or_default(),
                head_sha,
                updated_at,
            })
        })
        .collect()
}
```

> If `GhUser`'s field is not named `login`, adjust the `pr()` test helper and the `author` mapping to the actual field name (read the struct).

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p arkavo-mcp-tools pr_watch_tests`
Expected: PASS (3 tests).

- [ ] **Step 6: Add the `GitHubPrWatchTool`**

In `github_api.rs`, mirroring `GitHubPrListTool`:

```rust
pub struct GitHubPrWatchTool {
    schema: ToolSchema,
}

impl GitHubPrWatchTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_pr_watch".to_string(),
                aliases: None,
                description: "List a repository's OPEN pull requests with each PR's head SHA and last-update time, newest first. Pass `since` (an ISO-8601 UTC timestamp from a prior call) to get only PRs updated after it — use this to discover new or changed PRs to review. Args: owner, repo, since (optional).".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": { "type": "string", "description": "Repository owner / org login." },
                        "repo": { "type": "string", "description": "Repository name." },
                        "since": { "type": "string", "description": "Optional ISO-8601 UTC timestamp; return only PRs updated after this." }
                    },
                    "required": ["owner", "repo"]
                }),
            },
        }
    }
}

impl Default for GitHubPrWatchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Tool for GitHubPrWatchTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, params: serde_json::Value) -> crate::Result<serde_json::Value> {
        let owner = params
            .get("owner")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("github_pr_watch: missing 'owner'".into()))?;
        let repo = params
            .get("repo")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("github_pr_watch: missing 'repo'".into()))?;
        let since = params.get("since").and_then(|v| v.as_str());

        let gh = get_github_client().await?;
        let url = format!(
            "{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls?state=open&sort=updated&direction=desc&per_page=100"
        );
        let resp = github_request(gh, gh.client.get(&url), "github_pr_watch").await?;
        let prs: Vec<GhPullRequest> = resp.json().await.map_err(|e| {
            ToolError::Mcp(format!("github_pr_watch: parse PRs failed: {e}"))
        })?;

        let selected = select_updated_prs(prs, since);
        Ok(serde_json::json!({
            "owner": owner,
            "repo": repo,
            "count": selected.len(),
            "pull_requests": selected,
        }))
    }
}
```

> Match the exact way `GitHubPrListTool` obtains the client and builds the request (e.g. if it calls `gh.client.get(...)` vs a helper). Use the same `ToolError` variants that file already uses for missing params / HTTP / parse errors.

- [ ] **Step 7: Register the tool in `registry.rs`**

In `register_all`, in the GitHub tools group (near the existing `GitHubPrListTool` registration), add an import for `GitHubPrWatchTool` alongside the other `github_api` imports, then:

```rust
self.register("github_pr_watch", Box::new(GitHubPrWatchTool::new()));
```

- [ ] **Step 8: Build, full focused test, clippy**

Run: `cargo test -p arkavo-mcp-tools pr_watch_tests` (PASS, 3 tests)
Run: `cargo build -p arkavo-mcp-tools` (clean)
Run: `cargo clippy -p arkavo-mcp-tools -- -D warnings` (clean)

- [ ] **Step 9: Commit**

```bash
git add crates/arkavo-mcp-tools/src/github_api.rs crates/arkavo-mcp-tools/src/registry.rs
git commit -m "Add github_pr_watch MCP tool for PR discovery

Lists a repo's open PRs with head SHA + updated_at, newest first; optional
`since` returns only PRs updated after it. Stateless — the swarm owns the
cursor. Self-contained in arkavo-mcp-tools (reqwest, mirrors github_api.rs);
PR-selection is a pure, unit-tested function.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VyuNT2XyZuxLMxLgkYc6ZG"
```

---

## Self-Review

**Spec coverage (WS-B scope):**
- "github_pr_watch tool — new/changed PRs since a cursor (head-SHA dedup)" → Task 1: the tool returns `head_sha` + `updated_at` and filters by `since`; dedup/cursor is the caller's (stateless tool, per the refined design). ✓
- "Self-contained in arkavo-mcp-tools, no new crate dep, no cycle" → Task 1 uses the existing reqwest pattern; no `arkavo-github` dep. ✓
- "Tests: new PR detected / changed re-triggers / unchanged skipped (mocked — no network)" → the three `pr_watch_tests` on the pure `select_updated_prs`. ✓

**Placeholder scan:** No TBD/vague steps. The two "if the field/method name differs, adjust" notes are named, checkable verifications against real structs, not placeholders.

**Type consistency:** `select_updated_prs(Vec<GhPullRequest>, Option<&str>) -> Vec<PrSummary>` used identically in the test (Step 2), the impl (Step 4), and the tool (Step 6). `GhHead { sha }` / `PrSummary` fields consistent across all steps. Tool name `github_pr_watch` consistent between schema (Step 6), registration (Step 7), and the WS-A manifest grant.

**Deviation from spec noted:** spec said the tool dedups against a *stored* cursor; this plan makes the tool stateless (returns head_sha + accepts `since`) and leaves the cursor to the caller — cleaner, avoids state-threading, same capability. Flagged for the final whole-branch review.
