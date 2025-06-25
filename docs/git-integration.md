# Git Integration

Arkavo Edge includes comprehensive Git integration for managing code changes, branches, and remote operations.

## Architecture

The Git integration is built with security and portability in mind:

- **No OpenSSL dependency**: Uses rustls for all TLS operations
- **Pure Rust implementation**: Uses git2 library without OpenSSL features
- **Fallback to system git**: For HTTPS/SSH remote operations

## Features

### Core Operations
- Repository initialization with "main" as default branch
- File staging and committing
- Branch creation and switching
- Diff generation
- Status tracking
- Commit history

### Safety Features
- **RepoGuard**: Transaction wrapper with automatic rollback on failure
- **Path sanitization**: Prevents directory traversal attacks
- **Pre-commit validation**: Optional fmt, clippy, and test checks
- **Atomic operations**: All-or-nothing commits with validation

### Remote Operations
- Fetch, pull, push operations
- Automatic fallback to system git for HTTPS/SSH URLs
- Support for local/file-based remotes via native git2

## Usage

### CLI Integration

The Git functionality is integrated into the `arkavo apply` command:

```bash
# Apply changes with auto-generated commit message
arkavo apply

# Apply without committing
arkavo apply --no-commit

# Apply with custom message
arkavo apply --message "feat: add new feature"

# Apply and push to remote
arkavo apply --push

# Skip validation checks
arkavo apply --no-validate
```

### MCP Tools

When using `arkavo serve`, the following Git tools are available:

- `git_status`: Get repository status
- `git_diff`: View changes (staged/unstaged)
- `git_commit`: Create commits
- `git_branch`: Manage branches
- `git_log`: View commit history
- `git_remote`: Handle fetch/pull/push operations

## Implementation Details

### OpenSSL-Free Build

To ensure cross-platform compatibility and avoid OpenSSL dependencies:

1. git2 is configured without the `https` feature
2. curl is configured with `rustls` feature
3. Remote operations delegate to system git for HTTPS/SSH URLs

### Security

- All repository paths are sanitized to prevent directory traversal
- MCP tools validate paths before operations
- RepoGuard ensures atomic operations with rollback capability

## Limitations

- SSH operations require system git to be installed
- HTTPS operations require system git to be installed
- Direct libgit2 HTTPS support is disabled to avoid OpenSSL

For local repositories and file-based remotes, all operations work natively without system git.