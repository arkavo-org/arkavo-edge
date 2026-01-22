# AGENTS.md

## test-generator-agent
purpose: Generate comprehensive unit and integration tests to achieve 85%+ code coverage target
model:   glm-4.7-flash
listen:  0.0.0.0:8403

# Test Generator Agent
# Test types:
# - Unit tests (inline #[cfg(test)] modules)
# - Integration tests (tests/ directory)
# - Property-based tests
# - Regression tests for bug fixes

# Coverage targets:
# - 85% line coverage minimum
# - All public APIs tested
# - Error paths covered
# - Edge cases identified

# Test patterns:
# - Arrange-Act-Assert
# - Given-When-Then for BDD
# - Table-driven tests for multiple inputs

discovery:
  mdns: true
