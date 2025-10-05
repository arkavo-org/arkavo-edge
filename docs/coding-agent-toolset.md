# Coding Agent Toolset

Comprehensive MCP tools for top-tier AI coding agents in Arkavo Edge.

## Overview

Arkavo Edge provides industry-standard tooling for 2025-grade coding agents, following the north star of:
- **MCP-first**: All tools exposed as Model Context Protocol servers
- **Ephemeral sandboxes**: Isolated execution with resource quotas
- **Continuous evaluation**: SWE-bench benchmarking for objective metrics

## Phase 1: Code Search & Intelligence ✅

### `arkavo-code-search` crate

Three powerful tools for code analysis and refactoring:

#### 1. `codegrep_search` (ripgrep)
Fast repository-wide code search with regex patterns.

**Capabilities:**
- Pattern matching with full regex support
- Multiple output modes (content, files, count)
- Glob filtering by file type
- Context lines (before/after)
- Case-insensitive search
- Line numbers
- Result limiting

**Example:**
```json
{
  "pattern": "async fn",
  "path": ".",
  "output_mode": "files",
  "glob": ["*.rs", "*.ts"],
  "max_results": 50
}
```

#### 2. `struct_find_replace` (Comby)
Language-aware structural search and replace using templates.

**Capabilities:**
- Template patterns with holes: `:[variable]`
- Language-specific matching (Rust, Go, Python, JS, TS, Java, C, C++)
- In-place or preview mode
- Directory exclusions
- Case-sensitive/insensitive matching

**Example:**
```json
{
  "match_template": "unwrap()",
  "rewrite_template": "unwrap_or_default()",
  "language": "rust",
  "in_place": false,
  "exclude_dirs": ["target", "node_modules"]
}
```

#### 3. `syntax_tree` (tree-sitter)
AST parsing for syntax-aware code analysis.

**Capabilities:**
- Parse files to AST (Rust, Python, JavaScript, TypeScript, Go)
- Multiple output formats:
  - `tree`: S-expression representation
  - `nodes`: Structured JSON nodes
  - `captures`: Query-based pattern matching
- Precise position tracking (row/column)
- Tree-sitter query language support

**Example:**
```json
{
  "file_path": "src/main.rs",
  "language": "rust",
  "output_format": "captures",
  "query": "(function_item name: (identifier) @fn.name)"
}
```

## Phase 2: Security & Quality ✅

### `arkavo-mcp-tools` enhancements

Three security tools integrated into the MCP toolset:

#### 1. `sec_semgrep` - SAST scanning
Static Application Security Testing with Semgrep.

**Capabilities:**
- Multiple rule configurations (auto, p/security-audit, p/owasp-top-ten, p/cwe-top-25, p/ci)
- Severity filtering (ERROR, WARNING, INFO)
- Path exclusion support
- Performance metrics tracking
- JSON output with summary statistics

**Example:**
```json
{
  "path": "src",
  "config": "p/owasp-top-ten",
  "severity": ["ERROR", "WARNING"],
  "exclude": ["vendor/", "test/"]
}
```

#### 2. `deps_osv` - Dependency vulnerabilities
Vulnerability scanning with OSV-Scanner.

**Capabilities:**
- Lockfile scanning (Cargo.lock, package-lock.json, go.mod, etc.)
- Multiple output formats (json, table, sarif, markdown)
- Call analysis for reduced false positives (experimental)
- Recursive directory scanning
- Offline mode with local database
- Severity-based vulnerability grouping

**Example:**
```json
{
  "lockfile": "Cargo.lock",
  "format": "json",
  "call_analysis": true
}
```

#### 3. `sbom_syft` - SBOM generation
Software Bill of Materials generation with Syft.

**Capabilities:**
- Multiple SBOM formats (CycloneDX JSON/XML, SPDX JSON/TagValue, GitHub JSON)
- Container image analysis
- Multi-platform image support
- Path exclusions
- Cataloger selection (Cargo, npm, Go, etc.)
- Package statistics by type and language

**Example:**
```json
{
  "source": ".",
  "format": "cyclonedx-json",
  "scope": "squashed",
  "catalogers": ["cargo", "npm"]
}
```

## Phase 3: Test & Automation ✅

### `arkavo-browser` crate
Browser automation using Chrome DevTools Protocol via chromiumoxide.

**browser_cdp** - Chrome DevTools Protocol automation

**Capabilities:**
- Navigate to URLs with headless/headed modes
- Screenshot capture (PNG format)
- JavaScript evaluation
- HTML content extraction
- Console API monitoring
- Network request tracking
- Custom viewport configuration

**Example:**
```json
{
  "action": "screenshot",
  "url": "https://example.com",
  "screenshot_path": "/tmp/screenshot.png",
  "headless": true,
  "viewport": {"width": 1920, "height": 1080}
}
```

Compatible with [Chrome DevTools MCP Server](https://github.com/ChromeDevTools/chrome-devtools-mcp/)

### Multi-language test runner

**test_run** - Unified test execution across languages

**Capabilities:**
- Auto-detect framework from project structure
- Supported frameworks: pytest, jest, go test, cargo test, xcodebuild
- Test pattern filtering
- Verbose output mode
- Coverage reporting
- Parallel execution
- Structured results with pass/fail/skip counts
- Framework-specific argument passthrough

**Example:**
```json
{
  "framework": "auto",
  "path": ".",
  "pattern": "test_auth",
  "verbose": true,
  "coverage": true,
  "parallel": true
}
```

**Framework Detection:**
- Cargo.toml → cargo test
- go.mod → go test
- package.json → jest
- pytest.ini/setup.py → pytest
- *.xcodeproj/*.xcworkspace → xcodebuild

## Phase 4: GitHub Integration ✅

### Enhanced GitHub tools in `arkavo-mcp-tools`

Two new tools for GitHub CI/CD and code review workflows:

#### 1. `gh_checks` - GitHub Checks API
Create and update check runs with inline code annotations.

**Capabilities:**
- Create check runs with status (queued/in_progress/completed)
- Update existing check runs
- Set conclusions (success/failure/neutral/cancelled/skipped/timed_out/action_required)
- Post inline annotations (file:line:message with notice/warning/failure levels)
- List check runs for commits
- Custom output (title/summary/text)

**Example:**
```json
{
  "action": "create",
  "repo": "owner/repo",
  "name": "Code Quality",
  "head_sha": "abc123",
  "status": "completed",
  "conclusion": "success",
  "output": {
    "title": "All checks passed",
    "summary": "No issues found"
  },
  "annotations": [{
    "path": "src/main.rs",
    "start_line": 42,
    "end_line": 42,
    "annotation_level": "warning",
    "message": "Consider using Result instead of unwrap()"
  }]
}
```

#### 2. `gh_pr_review` - PR review with line comments
Submit pull request reviews with line-level feedback and code suggestions.

**Capabilities:**
- Submit reviews (COMMENT/APPROVE/REQUEST_CHANGES)
- Line-level code comments
- Side selection (LEFT for old code, RIGHT for new)
- Auto-fetch latest commit SHA
- Batch comments in single review

**Example:**
```json
{
  "repo": "owner/repo",
  "pr_number": 123,
  "event": "REQUEST_CHANGES",
  "body": "Please address the issues below",
  "comments": [{
    "path": "src/auth.rs",
    "line": 56,
    "body": "This function needs error handling",
    "side": "RIGHT"
  }]
}
```

## Phase 5: Ephemeral Workspaces ✅

### `arkavo-workspace` crate
Container-based ephemeral workspaces with resource quotas and isolation.

**workspace_container** - Isolated execution environments

**Capabilities:**
- Auto-detect runtime (Docker/Podman)
- Create isolated containers with resource limits
- Git repository cloning into workspace
- Execute commands with timeout enforcement
- CPU and memory quotas (--cpus, --memory)
- Network isolation (disabled by default)
- Environment variable injection
- Auto-cleanup on container removal
- List active workspaces

**Example - Create workspace:**
```json
{
  "action": "create",
  "workspace_id": "arkavo-workspace-task-123",
  "image": "ubuntu:22.04",
  "repo_url": "https://github.com/example/repo",
  "cpu_limit": "1.0",
  "memory_limit": "512m",
  "network": false,
  "env": {
    "RUST_BACKTRACE": "1"
  }
}
```

**Example - Execute command:**
```json
{
  "action": "execute",
  "workspace_id": "arkavo-workspace-task-123",
  "command": "cargo test",
  "timeout": 300
}
```

**Example - Cleanup:**
```json
{
  "action": "cleanup",
  "workspace_id": "arkavo-workspace-task-123"
}
```

**Architecture:**
- 375 LoC (under 400 limit) ✅
- Docker and Podman support
- Resource-limited execution
- Timeout enforcement
- Isolated network by default
- Production-ready error handling

## Phase 6: SWE-bench Evaluation ✅

### `arkavo-bench` crate
Objective benchmarking harness for coding agent evaluation using SWE-bench datasets.

**swe_bench** - Automated evaluation tool

**Capabilities:**
- Load instances from SWE-bench datasets (lite/verified/test)
- Containerized execution per instance
- Automatic git repository cloning
- Test-based resolution evaluation
- Comprehensive metrics tracking:
  - Resolved percentage
  - Wall-time per instance
  - API call counts
  - Token usage
  - Estimated cost (USD)
- Solution evaluation with patch application
- JSON metrics export/import
- Summary statistics generation

**Example - Load instances:**
```json
{
  "action": "load",
  "subset": "lite",
  "limit": 10
}
```

**Example - Run benchmark:**
```json
{
  "action": "run",
  "subset": "lite",
  "limit": 5,
  "metrics_file": "results/swe-bench-lite.json"
}
```

**Example - Evaluate solution:**
```json
{
  "action": "evaluate",
  "subset": "lite",
  "instance_id": "django__django-12345",
  "solution": "diff --git a/file.py..."
}
```

**Example - Generate summary:**
```json
{
  "action": "summary",
  "metrics_file": "results/swe-bench-lite.json"
}
```

**Metrics Tracked:**
- `resolved`: Boolean success indicator
- `wall_time_ms`: Execution time
- `api_calls`: Number of LLM calls
- `total_tokens`: Token usage
- `estimated_cost_usd`: Cost estimation
- `error_message`: Failure details

**Architecture:**
- 368 LoC bench.rs (under 400 limit) ✅
- 96 LoC metrics.rs ✅
- 30 LoC error.rs ✅
- Integrates with arkavo-workspace for isolation
- Async task execution
- Production-ready error handling

## Phase 7: MCP Consolidation

### Unified tool registry
- All tools as MCP servers (JSON-RPC)
- Standardized schemas
- Model-agnostic (Claude, GPT, Qwen, etc.)
- Tool discovery

## Usage

### As MCP Tools
```rust
use arkavo_code_search::{CodeGrepTool, CombyTool, TreeSitterTool};
use arkavo_mcp::Tool;

let grep = CodeGrepTool::new();
let result = grep.execute(params).await?;
```

### Via MCP Server
```bash
# Start MCP server with all tools
arkavo mcp --tools code-search,security,test
```

## Dependencies

External tools required:
- `rg` (ripgrep) - for codegrep_search
- `comby` - for struct_find_replace
- `semgrep` - for security scanning (Phase 2)
- `osv-scanner` - for dependency scanning (Phase 2)
- `syft` - for SBOM generation (Phase 2)
- `docker` or `podman` - for ephemeral workspaces (Phase 5)

## Architecture

Each capability follows Arkavo conventions:
- One crate per capability
- All files < 400 LoC
- Production-ready (no stubs/placeholders)
- MCP-compatible tool interface
- Full error handling

## Benchmarking

Track progress on SWE-bench:
```bash
arkavo bench swe --subset lite
arkavo bench swe --subset verified
arkavo bench swe --subset live
```

Results published to `docs/benchmarks/` with:
- Resolved percentage
- Wall-time per instance
- API cost tracking
- Comparison to baselines

## References

- [MCP Specification](https://modelcontextprotocol.io)
- [SWE-bench](https://www.swebench.com)
- [Comby](https://comby.dev)
- [tree-sitter](https://tree-sitter.github.io)
- [Semgrep](https://semgrep.dev)
