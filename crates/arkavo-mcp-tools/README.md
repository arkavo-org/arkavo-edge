# arkavo-mcp-tools

Unified MCP tool registry for Arkavo Edge coding agents.

## Overview

This crate provides a comprehensive set of MCP tools organized into categories:

### Security Tools
- **sec_semgrep** - SAST scanning with Semgrep
- **deps_osv** - Dependency vulnerability scanning with OSV-Scanner
- **sbom_syft** - SBOM generation with Syft

### GitHub Tools
- **gh_checks** - GitHub Checks API integration
- **gh_pr_review** - PR reviews with line-level comments
- **github_pr_create** - Create pull requests

### Testing Tools
- **test_run** - Multi-language test runner (pytest, jest, go test, cargo test, xcodebuild)

### Code Analysis Tools
- **code_analysis** - Code analysis and metrics

### File System Tools
- **filesystem** - File system operations

### Git Tools
- **git** - Git repository operations

## Tool Registry

The `ToolRegistry` provides centralized access to all tools:

```rust
use arkavo_mcp_tools::ToolRegistry;

let registry = ToolRegistry::new();

// List all tools
let tools = registry.list_tools();

// List tools by category
let by_category = registry.list_by_category();

// Get specific tool
if let Some(tool) = registry.get("sec_semgrep") {
    // Use tool
}

// Export tool schemas
let schemas = registry.export_schemas();
```

## Tool Categories

Tools are automatically categorized:
- **Security**: SAST, dependency scanning, SBOM
- **GitHub**: Checks API, PR reviews, PR creation
- **Testing**: Multi-language test execution
- **Code Analysis**: Code metrics and analysis
- **File System**: File operations
- **Git**: Repository operations

## External Dependencies

Some tools require external binaries:
- **semgrep** - `semgrep` CLI
- **osv-scanner** - `osv-scanner` CLI
- **syft** - `syft` CLI
- **gh_checks/gh_pr_review** - `gh` CLI (GitHub CLI)

## Usage

### As a Library

```rust
use arkavo_mcp_tools::ToolRegistry;

#[tokio::main]
async fn main() {
    let registry = ToolRegistry::new();

    // Get tool by name
    if let Some(semgrep) = registry.get("sec_semgrep") {
        let params = serde_json::json!({
            "path": "src",
            "config": "p/owasp-top-ten"
        });

        let result = semgrep.execute(params).await;
    }
}
```

### Schema Export

Export all tool schemas to JSON:

```rust
let registry = ToolRegistry::new();
let schemas = registry.export_schemas();
println!("{}", serde_json::to_string_pretty(&schemas).unwrap());
```

## Architecture

- All tools implement the `Tool` trait from `arkavo-mcp`
- Tools are registered in the `ToolRegistry` on initialization
- Tools are categorized automatically based on naming conventions
- Schemas are exportable in MCP-compatible JSON format
