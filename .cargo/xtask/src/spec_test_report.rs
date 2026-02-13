use crate::spec_test::{CoverageStatus, Criticality, SpecCoverage};

pub struct QualityGateResult {
    pub passed: bool,
    pub messages: Vec<String>,
}

pub fn format_markdown_report(
    specs: &[SpecCoverage],
    total_scenarios: usize,
    covered_scenarios: usize,
    coverage_pct: f64,
    fail_under: Option<f64>,
    critical_required: bool,
) -> (String, QualityGateResult) {
    let mut md = String::new();
    let mut gate = QualityGateResult {
        passed: true,
        messages: Vec::new(),
    };

    md.push_str("## Spec Coverage Report\n\n");
    md.push_str("<!-- spec-coverage-report -->\n\n");

    // Summary table
    md.push_str("| Spec | Scenarios | Covered | Partial | Missing | Coverage |\n");
    md.push_str("|------|-----------|---------|---------|---------|----------|\n");

    for spec_cov in specs {
        let name = spec_cov
            .file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .replace(".spec", "");
        let total = spec_cov.scenarios.len();
        let covered = spec_cov
            .scenarios
            .iter()
            .filter(|s| matches!(s.status, CoverageStatus::Covered))
            .count();
        let partial = spec_cov
            .scenarios
            .iter()
            .filter(|s| matches!(s.status, CoverageStatus::Partial))
            .count();
        let missing = spec_cov
            .scenarios
            .iter()
            .filter(|s| matches!(s.status, CoverageStatus::Missing))
            .count();
        let pct = if total > 0 {
            ((covered as f64 + partial as f64 * 0.5) / total as f64 * 100.0) as u32
        } else {
            0
        };
        md.push_str(&format!(
            "| {name} | {total} | {covered} | {partial} | {missing} | {pct}% |\n"
        ));
    }

    let total_partial: usize = specs
        .iter()
        .map(|s| {
            s.scenarios
                .iter()
                .filter(|sc| matches!(sc.status, CoverageStatus::Partial))
                .count()
        })
        .sum();
    let total_missing = total_scenarios - covered_scenarios - total_partial;

    md.push_str(&format!(
        "| **Total** | **{total_scenarios}** | **{covered_scenarios}** | **{total_partial}** | **{total_missing}** | **{coverage_pct:.1}%** |\n"
    ));

    // Quality gate section
    md.push_str("\n### Quality Gate\n\n");

    if let Some(threshold) = fail_under {
        if coverage_pct >= threshold {
            md.push_str(&format!(
                "- :white_check_mark: Overall coverage: {coverage_pct:.1}% (threshold: {threshold}%)\n"
            ));
        } else {
            md.push_str(&format!(
                "- :x: Overall coverage: {coverage_pct:.1}% (threshold: {threshold}%)\n"
            ));
            gate.passed = false;
            gate.messages.push(format!(
                "Coverage {coverage_pct:.1}% is below threshold {threshold}%"
            ));
        }
    }

    // Critical/high scenario check
    let mut uncovered_critical: Vec<(&str, &str, &str, &str)> = Vec::new();
    for spec_cov in specs {
        let spec_name = spec_cov
            .file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        for sc in &spec_cov.scenarios {
            let is_critical_or_high = matches!(
                sc.scenario.criticality,
                Criticality::Critical | Criticality::High
            );
            if is_critical_or_high && matches!(sc.status, CoverageStatus::Missing) {
                uncovered_critical.push((
                    &sc.scenario.id,
                    spec_name,
                    sc.scenario.criticality.as_str(),
                    &sc.scenario.name,
                ));
            }
        }
    }

    if critical_required {
        if uncovered_critical.is_empty() {
            md.push_str("- :white_check_mark: All critical/high scenarios have test coverage\n");
        } else {
            md.push_str(&format!(
                "- :x: {} critical/high scenarios lack test coverage\n",
                uncovered_critical.len()
            ));
            gate.passed = false;
            gate.messages.push(format!(
                "{} critical/high scenarios have no tests",
                uncovered_critical.len()
            ));
        }
    }

    // Uncovered scenarios detail
    let all_uncovered: Vec<(&str, &str, &str, &str)> = collect_uncovered(specs);
    if !all_uncovered.is_empty() {
        md.push_str(&format!(
            "\n<details>\n<summary>Uncovered Scenarios ({})</summary>\n\n",
            all_uncovered.len()
        ));
        md.push_str("| ID | Spec | Criticality | Scenario |\n");
        md.push_str("|----|------|-------------|----------|\n");
        for (id, spec, crit, name) in &all_uncovered {
            md.push_str(&format!("| {id} | {spec} | {crit} | {name} |\n"));
        }
        md.push_str("\n</details>\n");
    }

    (md, gate)
}

fn collect_uncovered(specs: &[SpecCoverage]) -> Vec<(&str, &str, &str, &str)> {
    let mut result = Vec::new();
    for spec_cov in specs {
        let spec_name = spec_cov
            .file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        for sc in &spec_cov.scenarios {
            if matches!(sc.status, CoverageStatus::Missing) {
                result.push((
                    sc.scenario.id.as_str(),
                    spec_name,
                    sc.scenario.criticality.as_str(),
                    sc.scenario.name.as_str(),
                ));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec_test::{ScenarioCoverage, Scenario, Spec, SpecCoverage};
    use std::path::PathBuf;

    fn make_scenario(id: &str, name: &str, crit: Criticality, status: CoverageStatus) -> ScenarioCoverage {
        ScenarioCoverage {
            scenario: Scenario {
                id: id.to_string(),
                name: name.to_string(),
                criticality: crit,
                given: vec![],
                when: String::new(),
                then: vec![],
                refs: vec![],
                edge_cases: vec![],
            },
            tests: vec![],
            status,
        }
    }

    fn make_spec(name: &str, scenarios: Vec<ScenarioCoverage>) -> SpecCoverage {
        SpecCoverage {
            spec: Spec {
                feature: name.to_string(),
                module: "test".to_string(),
                version: "1.0".to_string(),
                invariants: vec![],
                scenarios: scenarios.iter().map(|s| s.scenario.clone()).collect(),
            },
            file: PathBuf::from(format!("{name}.spec.yaml")),
            scenarios,
        }
    }

    #[test]
    fn markdown_report_contains_table_and_marker() {
        let specs = vec![make_spec("auth", vec![
            make_scenario("AUTH-1", "Login", Criticality::Critical, CoverageStatus::Covered),
            make_scenario("AUTH-2", "Logout", Criticality::High, CoverageStatus::Missing),
        ])];
        let (md, gate) = format_markdown_report(&specs, 2, 1, 50.0, Some(85.0), true);
        assert!(md.contains("<!-- spec-coverage-report -->"));
        assert!(md.contains("| auth |"));
        assert!(md.contains(":x: Overall coverage: 50.0%"));
        assert!(md.contains(":x: 1 critical/high scenarios"));
        assert!(!gate.passed);
    }

    #[test]
    fn quality_gate_passes_when_thresholds_met() {
        let specs = vec![make_spec("net", vec![
            make_scenario("NET-1", "Connect", Criticality::Critical, CoverageStatus::Covered),
        ])];
        let (md, gate) = format_markdown_report(&specs, 1, 1, 100.0, Some(85.0), true);
        assert!(md.contains(":white_check_mark:"));
        assert!(gate.passed);
    }

    #[test]
    fn uncovered_details_section_rendered() {
        let specs = vec![make_spec("io", vec![
            make_scenario("IO-1", "Read file", Criticality::Low, CoverageStatus::Missing),
        ])];
        let (md, _) = format_markdown_report(&specs, 1, 0, 0.0, None, false);
        assert!(md.contains("<details>"));
        assert!(md.contains("| IO-1 |"));
    }
}
