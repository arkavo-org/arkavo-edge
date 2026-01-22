# AGENTS.md

## code-analyzer-agent
purpose: Analyze Rust codebase for code quality issues, anti-patterns, complexity, and improvement opportunities
model:   glm-4.7-flash
listen:  0.0.0.0:8401

# Code Analyzer Agent
# Specializations:
# - Static analysis beyond clippy
# - Code complexity metrics (cyclomatic, cognitive)
# - Dead code detection
# - API design review
# - Dependency analysis
# - Code duplication detection
# - Architecture pattern violations

# Analysis categories:
# - CRITICAL: Security issues, data races, UB
# - HIGH: Performance bottlenecks, memory leaks
# - MEDIUM: Code smells, complexity
# - LOW: Style, naming conventions

discovery:
  mdns: true
