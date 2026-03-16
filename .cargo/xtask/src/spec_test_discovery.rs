use anyhow::Result;
use regex::Regex;
use std::path::Path;
use walkdir::WalkDir;

use crate::spec_test::Test;

pub struct TestDiscovery {
    spec_attr_pattern: Regex,
    legacy_pattern: Regex,
    doc_pattern: Regex,
    test_pattern: Regex,
}

impl TestDiscovery {
    pub fn new() -> Result<Self> {
        Ok(Self {
            spec_attr_pattern: Regex::new(r#"#\[spec\("([A-Z]+-\d+)"\)"#)?,
            legacy_pattern: Regex::new(r"Covers\s+([A-Z]+-\d+)")?,
            // Also match doc comment patterns like "Spec: SESS-001" or "SESS-001: description"
            doc_pattern: Regex::new(r"(?:Spec:\s+)?([A-Z]+-\d+):")?,
            test_pattern: Regex::new(r"(async\s+)?fn\s+(test_\w+)")?,
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

        for (line_num, line) in content.lines().enumerate() {
            for cap in self.spec_attr_pattern.captures_iter(line) {
                current_scenarios.push(cap[1].to_string());
            }
            for cap in self.legacy_pattern.captures_iter(line) {
                current_scenarios.push(cap[1].to_string());
            }
            for cap in self.doc_pattern.captures_iter(line) {
                current_scenarios.push(cap[1].to_string());
            }
            if let Some(cap) = self.test_pattern.captures(line) {
                tests.push(Test {
                    name: cap[2].to_string(),
                    file: path.to_path_buf(),
                    line: line_num + 1,
                    scenarios_covered: current_scenarios.clone(),
                    is_async: cap.get(1).is_some(),
                });
                current_scenarios.clear();
            }
        }
        Ok(tests)
    }
}
