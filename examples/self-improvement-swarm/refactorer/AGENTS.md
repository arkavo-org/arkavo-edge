# AGENTS.md

## refactorer-agent
purpose: Apply safe code refactoring transformations including extract function, inline, rename, and structural changes
model:   glm-4.7-flash
listen:  0.0.0.0:8402

# Refactorer Agent
# Safe transformations:
# - Extract function/method
# - Inline function/variable
# - Rename symbols
# - Extract trait/impl
# - Simplify conditionals
# - Remove dead code
# - Consolidate duplicate code

# Constraints:
# - Must preserve behavior (semantic equivalence)
# - Must pass existing tests after refactoring
# - Follows Rust idioms and conventions
# - Respects 400-line file limit from CLAUDE.md

discovery:
  mdns: true
