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

## Phase 2: Security & Quality (In Progress)

### `arkavo-mcp-tools` enhancements

#### 1. `sec.semgrep` - SAST scanning
- Security rule packs (OWASP, CWE)
- Language-specific patterns
- Custom rule support
- CI/local modes

#### 2. `deps.osv` - Dependency vulnerabilities
- OSV-Scanner integration
- SBOM-aware scanning
- Severity scoring
- Remediation suggestions

#### 3. `sbom.syft` - SBOM generation
- CycloneDX and SPDX formats
- Container image support
- Dependency graphs
- License detection

## Phase 3: Test & Automation

### `arkavo-browser` crate
- Playwright integration (Chromium/WebKit/Firefox)
- Screenshot/video recording
- Network mocking
- E2E test assertions

### Multi-language test runner
- Unified interface: `build.test`
- Support for pytest, jest, go test, cargo test
- iOS/macOS: xcodebuild orchestration
- Structured results (pass/fail/skip counts)

## Phase 4: GitHub Integration

### Enhanced `github.rs` in `arkavo-mcp-tools`

#### 1. `gh.checks` - GitHub Checks API
- Create/update check runs
- Post inline annotations (file:line:message)
- Conclusion states (success/failure/neutral)

#### 2. PR review enhancements
- Line-level review comments
- Request changes, approve, comment
- Code suggestions

## Phase 5: Ephemeral Workspaces

### `arkavo-workspace` crate
- Container-based isolation (Docker/Podman)
- Clone repo per task
- Read-only network with package allowlist
- CPU/RAM/time quotas
- Auto-cleanup

## Phase 6: SWE-bench Evaluation

### `arkavo-bench` crate
- SWE-bench Lite/Verified/Live harness
- Containerized execution per instance
- Metrics: Resolved %, wall-time, cost
- Plan-then-act loop tracking
- Nightly CI runs
- Dashboard in `docs/benchmarks/`

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
