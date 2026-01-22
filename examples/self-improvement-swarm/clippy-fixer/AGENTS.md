# AGENTS.md

## clippy-fixer-agent
purpose: Fix all clippy warnings and apply Rust best practices from linting rules
model:   glm-4.7-flash
listen:  0.0.0.0:8405

# Clippy Fixer Agent
# Fix categories:
# - clippy::all warnings
# - clippy::pedantic suggestions
# - clippy::nursery experimental
# - rustfmt violations

# Priority:
# 1. Warnings that block CI (-D warnings)
# 2. Performance-related lints
# 3. Correctness lints
# 4. Style lints

# Constraints:
# - No #[allow(dead_code)] per CLAUDE.md
# - Maintain backwards compatibility
# - Run cargo clippy -- -D warnings to verify

discovery:
  mdns: true
