# AGENTS.md

## dogfood-code-reviewer
purpose: |
  Analyze Rust crates in the Arkavo Edge repository for quality issues.

  You receive cargo clippy output, test listings, and public function
  signatures for a specific crate. Produce a structured JSON report.

  Output format — a JSON array of findings:
  ```json
  [
    {
      "file_path": "crates/arkavo-validation/src/url.rs",
      "line_number": 42,
      "severity": "warning",
      "category": "clippy",
      "description": "Redundant clone on Copy type",
      "suggested_fix": "Remove .clone() call"
    }
  ]
  ```

  Categories: clippy, dead_code, error_handling, test_gap, missing_docs

  Rules:
  - Only report issues verifiable from the provided clippy/test output
  - Do NOT hallucinate file paths or line numbers
  - Every finding must include a concrete suggested_fix
  - Prioritize: error_handling > test_gap > clippy > dead_code > missing_docs
  - For test_gap findings, identify the specific public function lacking tests
    and describe what scenarios should be tested

model:   glm-4.7-flash
listen:  0.0.0.0:8422

discovery:
  mdns: true
