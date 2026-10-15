use anyhow::Result;
use regex::Regex;
use std::path::Path;
use walkdir::WalkDir;

use crate::spec_test::Test;

pub struct TestDiscovery {
    spec_attr_pattern: Regex,
    spec_id_pattern: Regex,
    legacy_pattern: Regex,
    doc_pattern: Regex,
    test_fn_pattern: Regex,
    fn_pattern: Regex,
    test_attr_pattern: Regex,
    ignore_attr_pattern: Regex,
}

impl TestDiscovery {
    pub fn new() -> Result<Self> {
        Ok(Self {
            // The attribute has to be the first token on the line. An unanchored
            // match treats a doc-comment example, such as the one on
            // `arkavo_test_macros::spec`, as a real test.
            spec_attr_pattern: Regex::new(r"^\s*#\[(?:\w+::)*spec\(([^)]+)\)\]")?,
            spec_id_pattern: Regex::new(r#""([A-Z]+-\d+)""#)?,
            legacy_pattern: Regex::new(r"Covers\s+([A-Z]+-\d+)")?,
            // Also match doc comment patterns like "Spec: SESS-001" or "SESS-001: description"
            doc_pattern: Regex::new(r"(?:Spec:\s+)?([A-Z]+-\d+):")?,
            // Match fn names starting with test_ (always considered a test)
            test_fn_pattern: Regex::new(r"(async\s+)?fn\s+(test_\w+)")?,
            // Match any fn name (used when preceded by #[test] or #[tokio::test])
            fn_pattern: Regex::new(r"(async\s+)?fn\s+(\w+)")?,
            // Detect #[test] or #[tokio::test] attributes
            test_attr_pattern: Regex::new(r"#\[(tokio::)?test")?,
            // #[ignore] and #[ignore = "reason"] are not part of the default run.
            ignore_attr_pattern: Regex::new(r"#\[\s*ignore\b")?,
        })
    }

    pub fn discover_tests(&self, crates_dir: &Path) -> Result<Vec<Test>> {
        let mut tests = Vec::new();
        for entry in WalkDir::new(crates_dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("rs")
                && let Ok(file_tests) = self.parse_test_file(path)
            {
                tests.extend(file_tests);
            }
        }
        Ok(tests)
    }

    fn parse_test_file(&self, path: &Path) -> Result<Vec<Test>> {
        let content = std::fs::read_to_string(path)?;
        let mut tests = Vec::new();
        let mut current_scenarios = Vec::new();
        let mut saw_test_attr = false;
        let mut ignored = false;

        for (line_num, line) in content.lines().enumerate() {
            // "Spec: ID" notes live in comments and stay attached to the next
            // test. A comment is not itself an attribute or a function, so a
            // documented example cannot become a test.
            let is_comment = line.trim_start().starts_with("//");
            if !is_comment {
                for attr in self.spec_attr_pattern.captures_iter(line) {
                    for id in self.spec_id_pattern.captures_iter(&attr[1]) {
                        current_scenarios.push(id[1].to_string());
                    }
                }
                if self.ignore_attr_pattern.is_match(line) {
                    ignored = true;
                }
                if self.test_attr_pattern.is_match(line) {
                    saw_test_attr = true;
                }
            }
            for cap in self.legacy_pattern.captures_iter(line) {
                current_scenarios.push(cap[1].to_string());
            }
            for cap in self.doc_pattern.captures_iter(line) {
                current_scenarios.push(cap[1].to_string());
            }
            if is_comment {
                continue;
            }
            // Match fn test_* (always) or any fn preceded by #[test]/#[tokio::test]
            let matched = if let Some(cap) = self.test_fn_pattern.captures(line) {
                Some((cap.get(1).is_some(), cap[2].to_string()))
            } else if saw_test_attr {
                self.fn_pattern
                    .captures(line)
                    .map(|cap| (cap.get(1).is_some(), cap[2].to_string()))
            } else {
                None
            };
            if let Some((is_async, name)) = matched {
                // An ignored test is not part of the default run, so its IDs
                // must not move coverage, and must not leak onto the next test.
                if !ignored {
                    tests.push(Test {
                        name,
                        file: path.to_path_buf(),
                        line: line_num + 1,
                        scenarios_covered: current_scenarios.clone(),
                        is_async,
                    });
                }
                current_scenarios.clear();
                saw_test_attr = false;
                ignored = false;
            } else if self.fn_pattern.is_match(line) {
                ignored = false;
            }
        }
        Ok(tests)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(name: &str, source: &str) -> Vec<Test> {
        let dir =
            std::env::temp_dir().join(format!("xtask-discovery-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.rs");
        std::fs::write(&path, source).unwrap();
        let discovery = TestDiscovery::new().unwrap();
        let tests = discovery.parse_test_file(&path).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        tests
    }

    #[test]
    fn single_id_attribute_counts_one_id() {
        let tests = parse("single", "#[spec(\"ABC-001\")]\n#[test]\nfn test_a() {}\n");
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].scenarios_covered, vec!["ABC-001".to_string()]);
    }

    #[test]
    fn multi_id_attribute_counts_every_id() {
        // Regression: only the first ID of #[spec("A-1", "B-2")] was counted.
        let tests = parse(
            "multi",
            "#[spec(\"ABC-001\", \"DEF-002\")]\n#[test]\nfn test_a() {}\n",
        );
        assert_eq!(tests.len(), 1);
        assert_eq!(
            tests[0].scenarios_covered,
            vec!["ABC-001".to_string(), "DEF-002".to_string()]
        );
    }

    #[test]
    fn fully_qualified_attribute_counts_ids() {
        let tests = parse(
            "qualified",
            "#[arkavo_test_macros::spec(\"ABC-001\", \"DEF-002\")]\n#[test]\nfn test_a() {}\n",
        );
        assert_eq!(tests.len(), 1);
        assert_eq!(
            tests[0].scenarios_covered,
            vec!["ABC-001".to_string(), "DEF-002".to_string()]
        );
    }

    #[test]
    fn stacked_attributes_accumulate() {
        let tests = parse(
            "stacked",
            "#[spec(\"ABC-001\")]\n#[spec(\"DEF-002\")]\n#[test]\nfn test_a() {}\n",
        );
        assert_eq!(tests.len(), 1);
        assert_eq!(
            tests[0].scenarios_covered,
            vec!["ABC-001".to_string(), "DEF-002".to_string()]
        );
    }

    #[test]
    fn doc_comment_example_contributes_no_scenarios() {
        // Regression: the unanchored attribute regex recorded the
        // arkavo-test-macros doc example as a test of GOSSIP-005 and GOSSIP-006.
        let tests = parse(
            "docs",
            r#"
/// ```ignore
/// #[spec("GOSSIP-005", "GOSSIP-006")]
/// #[tokio::test]
/// async fn test_anti_entropy() { }
/// ```
#[spec("ABC-001")]
#[test]
fn test_real() {}
"#,
        );
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].name, "test_real");
        assert_eq!(tests[0].scenarios_covered, vec!["ABC-001".to_string()]);
    }

    #[test]
    fn ignored_test_does_not_count_and_does_not_leak() {
        let tests = parse(
            "ignore",
            "#[spec(\"ABC-001\")]\n#[test]\n#[ignore]\nfn test_skipped() {}\n#[spec(\"DEF-002\")]\n#[test]\nfn test_kept() {}\n",
        );
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].name, "test_kept");
        assert_eq!(tests[0].scenarios_covered, vec!["DEF-002".to_string()]);
    }

    #[test]
    fn ignore_with_reason_does_not_count() {
        let tests = parse(
            "ignore-reason",
            "#[spec(\"ABC-001\")]\n#[ignore = \"needs network\"]\n#[tokio::test]\nasync fn test_skipped() {}\n",
        );
        assert!(tests.is_empty());
    }
}
