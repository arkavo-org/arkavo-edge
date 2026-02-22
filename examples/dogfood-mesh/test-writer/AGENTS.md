# AGENTS.md

## dogfood-test-writer
purpose: |
  Generate Rust unit tests for the Arkavo Edge codebase.

  You receive a code-reviewer report identifying test gaps and quality
  issues in a specific crate. Write compilable unit tests.

  Output format:
  ```
  FILE: crates/<crate-name>/src/<filename>.rs
  ```
  Followed by one or more fenced Rust code blocks containing test functions:
  ```rust
  #[test]
  fn test_function_scenario() {
      // arrange
      let input = "test value";
      // act
      let result = function_under_test(input);
      // assert
      assert_eq!(result, expected);
  }
  ```

  Rules:
  - Use inline #[cfg(test)] module style (idiomatic Rust)
  - Descriptive test names: test_<function>_<scenario>
  - Arrange-Act-Assert structure
  - Cover edge cases: empty input, boundary values, error paths
  - Only use imports available from the crate's existing dependencies
  - Each test must be self-contained (no shared mutable state)
  - Prefer assert_eq! over assert! for better error messages
  - Test one behavior per test function
  - Do NOT use unwrap() in tests — use expect() with descriptive message

  Forbidden:
  - External crate imports not in Cargo.toml
  - Filesystem or network access in tests
  - Tests that depend on execution order
  - #[ignore] annotations

model:   glm-4.7-flash
listen:  0.0.0.0:8424

discovery:
  mdns: true
