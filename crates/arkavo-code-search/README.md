# arkavo-code-search

Fast code search tools for Arkavo Edge coding agents.

## Features

### CodeGrep (ripgrep integration)
Fast repository-wide code search with support for:
- Pattern matching with regex
- Multiple output modes (content, files, count)
- Glob filtering (e.g., `*.rs`, `*.ts`)
- Context lines (-A/-B)
- Case-insensitive search
- Line numbers
- Result limits

### Comby (structural search/replace)
Language-aware structural refactoring:
- Template-based patterns with holes (`:[ var]`)
- Rewrite templates
- Language-specific matching (Rust, Go, Python, JS, TS, Java, C, C++)
- In-place or preview mode
- Directory exclusions

### TreeSitter (AST parsing)
Syntax-aware code analysis:
- Parse files to AST (Rust, Python, JavaScript, TypeScript, Go)
- Multiple output formats (tree, nodes, captures)
- Tree-sitter query support for pattern matching
- Precise node positions for surgical edits

## Usage

```rust
use arkavo_code_search::CodeGrepTool;
use arkavo_mcp::Tool;
use serde_json::json;

#[tokio::main]
async fn main() {
    let tool = CodeGrepTool::new();

    // Search for async functions in Rust files
    let params = json!({
        "pattern": "async fn",
        "path": ".",
        "output_mode": "files",
        "glob": ["*.rs"]
    });

    let result = tool.execute(params).await.unwrap();
    println!("{}", result);
}
```

## Tool Schema

### `codegrep_search`

Search for code patterns using ripgrep.

**Parameters:**
- `pattern` (required): Regular expression pattern to search for
- `path`: Directory or file to search (default: current directory)
- `glob`: Array of glob patterns to filter files
- `output_mode`: "content", "files", or "count" (default: "files")
- `case_insensitive`: Boolean for case-insensitive search
- `context_before`: Number of lines to show before match
- `context_after`: Number of lines to show after match
- `line_numbers`: Show line numbers (content mode only)
- `max_results`: Maximum number of results

**Returns:**
```json
{
  "mode": "files",
  "count": 42,
  "files": ["src/lib.rs", "src/main.rs"]
}
```

## Dependencies

Requires `rg` (ripgrep) to be installed and available in PATH.

```bash
# macOS
brew install ripgrep

# Ubuntu/Debian
apt-get install ripgrep

# Arch Linux
pacman -S ripgrep
```

## Roadmap

- [ ] Comby structural search/replace
- [ ] tree-sitter AST parsing
- [ ] Language-aware symbol search
