# Git Integration Enhancement for Arkavo Edge

## Overview

Implement comprehensive Git functionality for Arkavo Edge to enable version control operations, automated commit management, and seamless integration with the agent workflow.

## Requirements

### Core Git Operations
- [x] Repository initialization and detection
- [x] Status checking with file tracking states
- [x] Staging and committing changes
- [x] Branch creation and switching
- [x] Unified diff generation with syntax highlighting
- [x] Remote operations (fetch, push, pull)

### Safety Features
- [x] Pre-commit validation hooks
- [x] Atomic commit transactions with rollback
- [x] Automatic backup before destructive operations
- [x] Conflict detection and resolution helpers
- [x] Respect .gitignore patterns

### Automation Features
- [x] Smart commit message generation
- [x] Feature branch naming conventions (arkavo/<timestamp> or feature/<name>)
- [x] Co-author attribution for AI-generated commits
- [x] Commit templates support

### Integration Points
- [x] `arkavo apply` - Auto-commit after code changes
- [x] `arkavo chat` - Show diff context during conversations
- [x] Repository mapper - Honor .gitignore patterns
- [x] Git config integration

## Implementation Plan

### Phase 1: Backend Abstraction (Week 1)
1. **Git Backend Trait**
   ```rust
   pub trait GitBackend {
       fn init(&self, path: &Path) -> Result<Repository>;
       fn status(&self, repo: &Repository) -> Result<Status>;
       fn add(&self, repo: &Repository, paths: &[&Path]) -> Result<()>;
       fn commit(&self, repo: &Repository, message: &str) -> Result<Oid>;
       fn create_branch(&self, repo: &Repository, name: &str) -> Result<Branch>;
       fn checkout(&self, repo: &Repository, name: &str) -> Result<()>;
       fn diff(&self, repo: &Repository, options: DiffOptions) -> Result<Diff>;
       fn fetch(&self, repo: &Repository, remote: &str) -> Result<()>;
       fn push(&self, repo: &Repository, remote: &str, branch: &str) -> Result<()>;
       fn rollback(&self, repo: &Repository, commit: Oid) -> Result<()>;
   }
   ```

2. **Git2 Implementation**
   - Default implementation using `git2` crate
   - Feature flag: `backend-git2` (default)

3. **Gitoxide Stub**
   - Future-ready implementation stub
   - Feature flag: `backend-gitoxide` (experimental)

### Phase 2: Core Operations (Week 2)
1. **Repository Management**
   - Auto-detect .git directory with work-tree support
   - Initialize new repositories
   - Status reporting with file states

2. **Commit Operations**
   - Stage files (individual, patterns, all)
   - Create commits with validation
   - Undo/amend last commit

3. **Branch Management**
   - Create branches with naming conventions
   - Switch branches with dirty check
   - List branches with current indicator

4. **Diff Generation**
   - Unified diff format
   - Syntax highlighting via `syntect`
   - Options: staged, unstaged, cached
   - Truncation for large diffs (>300 lines)

### Phase 3: Safety Layer (Week 3)
1. **RepoGuard Transaction System**
```rust
   pub struct RepoGuard<'a> {
       repo: &'a Repository,
       backup: Option<Oid>,
   }
   
   impl<'a> RepoGuard<'a> {
       pub fn transaction<F>(&mut self, f: F) -> Result<Oid>
       where F: FnOnce(&Repository) -> Result<Oid>
   }
```

2. **Pre-commit Validation**
   - Run `cargo fmt --check`
   - Run `cargo clippy -- -D warnings`
   - Custom hooks support
   - Automatic rollback on failure

3. **Conflict Helpers**
   - Detect merge conflicts
   - Provide resolution suggestions
   - Three-way merge support

### Phase 4: Remote Operations & Integration (Week 4)
1. **Remote Workflow**
   - `sync_upstream()` - fetch + rebase
   - `publish()` - push with upstream tracking
   - Credential helper integration

2. **Command Integration**
   - `arkavo apply`:
     - Stage all changes
     - Generate commit message
     - Commit with validation
     - Optional push
   - `arkavo chat`:
     - Show current diff context
     - Reference commit history
   - Repository mapper:
     - Parse .gitignore
     - Exclude ignored files

3. **Commit Message Generation**
   - Analyze diff for change type (feat/fix/refactor)
   - Generate descriptive message
   - Add AI attribution suffix
   - Support `--edit` flag for manual editing

## Testing Strategy

### Unit Tests
- Mock GitBackend for isolated testing
- Test each operation in isolation
- Cover error paths and edge cases

### Integration Tests
```rust
#[test]
fn test_full_workflow() {
    let temp_dir = tempfile::tempdir().unwrap();
    let repo = init_repo(&temp_dir.path()).unwrap();
    
    // Create file
    fs::write(temp_dir.path().join("test.txt"), "content").unwrap();
    
    // Stage and commit
    add_all(&repo).unwrap();
    let oid = commit(&repo, "test commit").unwrap();
    
    // Verify
    assert_eq!(repo.head().unwrap().target().unwrap(), oid);
}
```

### CLI Tests
- Use `assert_cmd` for end-to-end testing
- Test conflict scenarios
- Verify rollback behavior

## Dependencies

### Required
- `git2 = "0.18"` - Git operations (MIT/Apache-2.0)
- `syntect = "5.0"` - Syntax highlighting
- `tempfile = "3.0"` - Testing

### Optional
- `gitoxide` - Future alternative backend

### System Dependencies
- macOS: `brew install libgit2`
- Ubuntu: `apt-get install libgit2-dev`
- Note: Consider static linking for distribution

## Success Criteria

1. **Functionality**
   - All git operations work correctly
   - No data loss or corruption
   - Performance comparable to git CLI

2. **User Experience**
   - Intuitive command integration
   - Clear diff previews
   - Helpful error messages

3. **Code Quality**
   - 85%+ test coverage
   - No clippy warnings
   - All files <400 LoC

4. **Safety**
   - Pre-commit validation prevents bad commits
   - Rollback always available
   - No destructive operations without confirmation

## Timeline

- Week 1: Backend abstraction + basic operations
- Week 2: Branch management + diff generation
- Week 3: Safety layer + remote operations
- Week 4: Command integration + testing

## Related Issues
- Builds on #9 (original Git integration request)
- Supports #8 (apply command implementation)
- Enhances #7 (repository mapping)