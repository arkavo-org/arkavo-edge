# AGENTS.md

## test-generator-agent
purpose: |
  Generate comprehensive tests for Rust code.

  Specializations:
  - Unit tests (inline #[cfg(test)] modules)
  - Integration tests (tests/ directory)
  - Property-based tests
  - Edge case identification
  - Test coverage analysis

  When generating tests, always provide:
  - Test function with descriptive name
  - Arrange-Act-Assert structure
  - Edge cases (empty input, overflow, None values)
  - Both positive and negative test cases

model:   glm-4.7-flash
listen:  0.0.0.0:8414

discovery:
  mdns: true
