#![allow(dead_code)] // Fields/methods used by spec_test_cmds
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub use crate::spec_test_discovery::TestDiscovery;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Spec {
    pub feature: String,
    pub module: String,
    pub version: String,
    #[serde(default)]
    pub invariants: Vec<String>,
    pub scenarios: Vec<Scenario>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Scenario {
    pub id: String,
    pub name: String,
    pub criticality: Criticality,
    #[serde(default)]
    pub given: Vec<String>,
    pub when: String,
    #[serde(default)]
    pub then: Vec<String>,
    #[serde(default)]
    pub refs: Vec<String>,
    #[serde(default)]
    pub edge_cases: Vec<EdgeCase>,
    #[serde(default)]
    pub wip: bool,
    #[serde(default)]
    pub issue: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EdgeCase {
    pub condition: String,
    pub expected: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Criticality {
    Critical,
    High,
    Medium,
    Low,
}

impl Criticality {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    pub fn priority(&self) -> u8 {
        match self {
            Self::Critical => 4,
            Self::High => 3,
            Self::Medium => 2,
            Self::Low => 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Test {
    pub name: String,
    pub file: PathBuf,
    pub line: usize,
    pub scenarios_covered: Vec<String>,
    pub is_async: bool,
}

#[derive(Debug, Clone)]
pub struct ScenarioCoverage {
    pub scenario: Scenario,
    pub tests: Vec<Test>,
    pub status: CoverageStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageStatus {
    Covered,
    Partial,
    Missing,
}

impl CoverageStatus {
    pub fn emoji(&self) -> &'static str {
        match self {
            Self::Covered => "🟢",
            Self::Partial => "🟡",
            Self::Missing => "🔴",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CoverageReport {
    pub specs: Vec<SpecCoverage>,
    pub total_scenarios: usize,
    pub covered_scenarios: usize,
    pub partial_scenarios: usize,
    pub missing_scenarios: usize,
}

#[derive(Debug, Clone)]
pub struct SpecCoverage {
    pub spec: Spec,
    pub file: PathBuf,
    pub scenarios: Vec<ScenarioCoverage>,
}

impl CoverageReport {
    pub fn coverage_percentage(&self) -> f64 {
        if self.total_scenarios == 0 {
            return 0.0;
        }
        (self.covered_scenarios as f64 / self.total_scenarios as f64) * 100.0
    }
}

// --- SpecParser ---

pub struct SpecParser;

impl SpecParser {
    pub fn parse_file(path: &Path) -> Result<Spec> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("reading spec: {}", path.display()))?;
        serde_yaml::from_str(&content)
            .with_context(|| format!("parsing spec YAML: {}", path.display()))
    }

    pub fn parse_all_specs(specs_dir: &Path) -> Result<Vec<(PathBuf, Spec)>> {
        let mut specs = Vec::new();
        for entry in std::fs::read_dir(specs_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) == Some("yaml")
                && path.file_name().and_then(|s| s.to_str()) != Some("index.yaml")
            {
                match Self::parse_file(&path) {
                    Ok(spec) => specs.push((path, spec)),
                    Err(e) => eprintln!("Warning: skipping {}: {e}", path.display()),
                }
            }
        }
        Ok(specs)
    }
}

// --- RefsValidator ---

#[derive(Debug, Clone)]
pub struct StaleRef {
    pub scenario_id: String,
    pub raw_ref: String,
    pub resolved_path: PathBuf,
    pub wip: bool,
}

#[derive(Debug, Clone)]
pub struct SpecStaleRefs {
    pub spec_file: PathBuf,
    pub stale_refs: Vec<StaleRef>,
}

pub struct RefsValidator;

impl RefsValidator {
    pub fn validate(specs: &[(PathBuf, Spec)]) -> Vec<SpecStaleRefs> {
        let mut result = Vec::new();
        for (spec_file, spec) in specs {
            let mut stale = Vec::new();
            for scenario in &spec.scenarios {
                // Carry the last explicit file path across refs in the same scenario
                // so that bare line-range entries (e.g., "141-151") reuse the prior path.
                let mut current_path: Option<String> = None;
                for raw_ref in &scenario.refs {
                    let parts: Vec<&str> = raw_ref.split(',').map(|s| s.trim()).collect();
                    for part in parts {
                        if part.is_empty() {
                            continue;
                        }
                        let path = if part.contains('/')
                            || part.contains('.')
                            || !Self::is_line_range(part)
                        {
                            Self::extract_path(part)
                        } else if let Some(ref _path) = current_path {
                            // Bare line range continuing previous path
                            continue;
                        } else {
                            part.to_string()
                        };
                        current_path = Some(path.clone());
                        let path_obj = Path::new(&path);
                        if !path_obj.exists() {
                            stale.push(StaleRef {
                                scenario_id: scenario.id.clone(),
                                raw_ref: raw_ref.clone(),
                                resolved_path: path_obj.to_path_buf(),
                                wip: scenario.wip,
                            });
                        }
                    }
                }
            }
            if !stale.is_empty() {
                result.push(SpecStaleRefs {
                    spec_file: spec_file.clone(),
                    stale_refs: stale,
                });
            }
        }
        result
    }

    fn extract_path(part: &str) -> String {
        // Split off line-range marker first, then strip any trailing
        // annotation like " (foo)" or " [bar]".
        let before_colon = part.find(':').map(|idx| &part[..idx]).unwrap_or(part);
        let stripped = if let Some((p, _)) = before_colon.split_once(" (") {
            p
        } else if let Some((p, _)) = before_colon.split_once(" [") {
            p
        } else {
            before_colon
        }
        .trim();
        stripped.to_string()
    }

    fn is_line_range(s: &str) -> bool {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '-')
    }
}

// --- CoverageAnalyzer ---

pub struct CoverageAnalyzer;

impl CoverageAnalyzer {
    pub fn analyze(specs: Vec<(PathBuf, Spec)>, tests: Vec<Test>) -> CoverageReport {
        let mut tests_by_scenario: HashMap<String, Vec<Test>> = HashMap::new();
        for test in tests {
            for scenario in &test.scenarios_covered {
                tests_by_scenario
                    .entry(scenario.clone())
                    .or_default()
                    .push(test.clone());
            }
        }

        let mut spec_coverages = Vec::new();
        let (mut total, mut covered, mut partial, mut missing) = (0, 0, 0, 0);

        for (file, spec) in specs {
            let mut scenario_coverages = Vec::new();
            for scenario in &spec.scenarios {
                total += 1;
                let scenario_tests = tests_by_scenario
                    .get(&scenario.id)
                    .cloned()
                    .unwrap_or_default();
                let status = match scenario_tests.len() {
                    0 => {
                        missing += 1;
                        CoverageStatus::Missing
                    }
                    1 => {
                        partial += 1;
                        CoverageStatus::Partial
                    }
                    _ => {
                        covered += 1;
                        CoverageStatus::Covered
                    }
                };
                scenario_coverages.push(ScenarioCoverage {
                    scenario: scenario.clone(),
                    tests: scenario_tests,
                    status,
                });
            }
            spec_coverages.push(SpecCoverage {
                spec,
                file,
                scenarios: scenario_coverages,
            });
        }

        CoverageReport {
            specs: spec_coverages,
            total_scenarios: total,
            covered_scenarios: covered,
            partial_scenarios: partial,
            missing_scenarios: missing,
        }
    }
}

// --- TestGenerator ---

pub struct TestGenerator;

impl TestGenerator {
    pub fn generate_stub(spec: &Spec, scenario: &Scenario) -> String {
        let fn_name = Self::scenario_to_fn_name(scenario);
        let module = spec.module.replace("::", "_");
        format!(
            r#"#[spec("{id}")]
/// {name}
/// Spec: specs/arkavo-edge/{module}.spec.yaml
/// Criticality: {crit}
#[tokio::test]
async fn {fn_name}() {{
    // TODO: Arrange - Set up preconditions
    // Given: {given}

    // TODO: Act - Execute the action
    // When: {when}

    // TODO: Assert - Verify expected outcomes
    // Then: {then}

    unimplemented!("Test stub for {id} - implement based on spec");
}}
"#,
            id = scenario.id,
            name = scenario.name,
            module = module,
            crit = scenario.criticality.as_str(),
            given = scenario.given.join(", "),
            when = scenario.when,
            then = scenario.then.join(", "),
        )
    }

    fn scenario_to_fn_name(scenario: &Scenario) -> String {
        let sanitized = scenario
            .name
            .to_lowercase()
            .replace(' ', "_")
            .replace(|c: char| !c.is_alphanumeric() && c != '_', "");
        format!(
            "test_{}_{}",
            scenario.id.to_lowercase().replace('-', "_"),
            sanitized
        )
    }

    pub fn generate_full_module(spec: &Spec, uncovered_only: bool, tests: &[Test]) -> String {
        let mut output = format!(
            "//! Auto-generated tests for {f}\n//! Module: {m}\n\n",
            f = spec.feature,
            m = spec.module,
        );
        let covered: HashSet<String> = tests
            .iter()
            .flat_map(|t| t.scenarios_covered.clone())
            .collect();
        for scenario in &spec.scenarios {
            if uncovered_only && covered.contains(&scenario.id) {
                continue;
            }
            output.push_str(&Self::generate_stub(spec, scenario));
            output.push('\n');
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_spec() {
        let yaml = r#"
feature: Test Feature
module: test_module
version: 0.1.0
invariants:
  - Test invariant
scenarios:
  - id: TEST-001
    name: Test scenario
    criticality: high
    given:
      - System is ready
    when: Action is triggered
    then:
      - Expected result occurs
"#;
        let spec: Spec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.feature, "Test Feature");
        assert_eq!(spec.scenarios.len(), 1);
        assert_eq!(spec.scenarios[0].id, "TEST-001");
    }

    #[test]
    fn test_coverage_status_and_criticality() {
        assert_eq!(CoverageStatus::Covered.emoji(), "🟢");
        assert_eq!(CoverageStatus::Partial.emoji(), "🟡");
        assert_eq!(CoverageStatus::Missing.emoji(), "🔴");
        assert_eq!(Criticality::Critical.priority(), 4);
        assert_eq!(Criticality::Low.priority(), 1);
    }

    #[test]
    fn test_refs_validator_skips_bare_line_ranges() {
        let spec = Spec {
            feature: "Test".into(),
            module: "test".into(),
            version: "0.1.0".into(),
            invariants: vec![],
            scenarios: vec![Scenario {
                id: "TEST-001".into(),
                name: "Test".into(),
                criticality: Criticality::High,
                given: vec![],
                when: "action".into(),
                then: vec![],
                refs: vec![
                    "crates/arkavo-crypto/src/lib.rs:335-339".into(),
                    "350-355".into(),
                ],
                edge_cases: vec![],
                wip: false,
                issue: None,
            }],
        };
        let stale = RefsValidator::validate(&[(PathBuf::from("test.spec.yaml"), spec)]);
        // The crypto lib.rs file may or may not exist; we only care that the bare
        // line range 350-355 is not reported as a missing path.
        for spec_stale in &stale {
            for r in &spec_stale.stale_refs {
                assert!(
                    r.resolved_path.to_str() != Some("350-355"),
                    "bare line range should not be reported as missing path: {:?}",
                    r
                );
            }
        }
    }

    #[test]
    fn test_generate_stub() {
        let spec = Spec {
            feature: "Test".into(),
            module: "test_mod".into(),
            version: "0.1.0".into(),
            invariants: vec![],
            scenarios: vec![],
        };
        let scenario = Scenario {
            id: "TEST-001".into(),
            name: "Basic test".into(),
            criticality: Criticality::High,
            given: vec!["A system".into()],
            when: "action occurs".into(),
            then: vec!["result happens".into()],
            refs: vec![],
            edge_cases: vec![],
            wip: false,
            issue: None,
        };
        let stub = TestGenerator::generate_stub(&spec, &scenario);
        assert!(stub.contains(r#"#[spec("TEST-001")]"#));
        assert!(stub.contains("test_test_001_basic_test"));
    }
}
