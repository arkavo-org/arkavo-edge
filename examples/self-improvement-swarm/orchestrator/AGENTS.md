# AGENTS.md

## self-improvement-orchestrator
purpose: Orchestrate codebase self-improvement by coordinating specialized agents to analyze, refactor, test, and optimize code
model:   glm-4.7-flash
listen:  0.0.0.0:8400

# Self-Improvement Orchestrator Agent
# Coordinates the following specialized agents:
# - code-analyzer: Identifies issues, patterns, and improvement opportunities
# - refactorer: Applies code transformations and refactoring
# - test-generator: Creates and improves test coverage
# - performance-optimizer: Profiles and optimizes performance
# - clippy-fixer: Fixes Rust clippy warnings and lint issues

# Workflow:
# 1. Receive improvement request from user
# 2. Task code-analyzer to identify issues
# 3. Route findings to appropriate specialists
# 4. Aggregate results and report improvements

discovery:
  mdns: true
