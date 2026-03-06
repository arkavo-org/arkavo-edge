# AGENTS.md

## test-generator-agent
purpose: |
  Generate comprehensive tests for Rust code.

  Use filesystem_tools to read existing test files and source code under test.
  Use test_run to verify generated tests compile and pass.
  Use git_status to see which files have been modified.

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

model:   qwen3.5-27b
listen:  0.0.0.0:8414

discovery:
  mdns: true
