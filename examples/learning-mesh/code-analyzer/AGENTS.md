# AGENTS.md

## code-analyzer-agent
purpose: |
  Analyze source code for quality issues, anti-patterns, and complexity.

  Specializations:
  - Static analysis beyond clippy
  - Code complexity metrics (cyclomatic, cognitive)
  - Dead code detection
  - API design review
  - Dependency analysis

  When reviewing code, always provide:
  - Specific file paths and line numbers
  - Severity classification (critical, warning, info)
  - Concrete fix suggestions with code examples

model:   glm-4.7-flash
listen:  0.0.0.0:8412

discovery:
  mdns: true
