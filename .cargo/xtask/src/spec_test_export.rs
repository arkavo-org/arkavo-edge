use crate::spec_test::{CoverageReport, CoverageStatus, SpecCoverage};
use anyhow::Result;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Serialize, serde::Deserialize)]
pub(crate) struct ExportData {
    pub(crate) specs: Vec<ExportSpec>,
    pub(crate) code: Vec<ExportCrate>,
    pub(crate) links: Vec<ExportLink>,
    pub(crate) summary: ExportSummary,
}

#[derive(Serialize, serde::Deserialize)]
pub(crate) struct ExportSpec {
    pub(crate) name: String,
    pub(crate) module: String,
    pub(crate) scenarios: Vec<ExportScenario>,
}

#[derive(Serialize, serde::Deserialize)]
pub(crate) struct ExportScenario {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) criticality: String,
    pub(crate) status: String,
    pub(crate) test_count: usize,
    pub(crate) refs: Vec<String>,
    pub(crate) given: Vec<String>,
    pub(crate) when: String,
    pub(crate) then: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) wip: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) issue: Option<String>,
}

#[derive(Serialize, serde::Deserialize)]
pub(crate) struct ExportCrate {
    name: String,
    files: Vec<ExportFile>,
}

#[derive(Serialize, serde::Deserialize)]
pub(crate) struct ExportFile {
    path: String,
    scenarios: Vec<String>,
}

#[derive(Serialize, serde::Deserialize)]
pub(crate) struct ExportLink {
    scenario: String,
    file: String,
    kind: String,
}

#[derive(Serialize, serde::Deserialize)]
pub(crate) struct ExportSummary {
    pub(crate) total: usize,
    pub(crate) covered: usize,
    pub(crate) partial: usize,
    pub(crate) missing: usize,
    #[serde(default)]
    pub(crate) wip: usize,
    pub(crate) pct: f64,
}

fn status_str(s: CoverageStatus) -> &'static str {
    match s {
        CoverageStatus::Covered => "covered",
        CoverageStatus::Partial => "partial",
        CoverageStatus::Missing => "missing",
    }
}

fn spec_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .replace(".spec", "")
}

pub fn export_json_string(report: &CoverageReport) -> Result<String> {
    let mut links = Vec::new();
    let mut code_map: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();

    let specs: Vec<ExportSpec> = report
        .specs
        .iter()
        .map(|sc| build_export_spec(sc, &mut links, &mut code_map))
        .collect();

    let code = build_code_tree(code_map);
    let pct = report.coverage_percentage();

    let wip_count: usize = report
        .specs
        .iter()
        .flat_map(|s| s.scenarios.iter())
        .filter(|s| s.scenario.wip)
        .count();

    let data = ExportData {
        specs,
        code,
        links,
        summary: ExportSummary {
            total: report.total_scenarios,
            covered: report.covered_scenarios,
            partial: report.partial_scenarios,
            missing: report.missing_scenarios.saturating_sub(wip_count),
            wip: wip_count,
            pct,
        },
    };

    Ok(serde_json::to_string_pretty(&data)?)
}

pub fn export_json(report: &CoverageReport, output: &Path) -> Result<()> {
    let json = export_json_string(report)?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, json)?;
    Ok(())
}

fn build_export_spec(
    sc: &SpecCoverage,
    links: &mut Vec<ExportLink>,
    code_map: &mut HashMap<String, HashMap<String, Vec<String>>>,
) -> ExportSpec {
    let scenarios = sc
        .scenarios
        .iter()
        .map(|cov| {
            for r in &cov.scenario.refs {
                links.push(ExportLink {
                    scenario: cov.scenario.id.clone(),
                    file: r.clone(),
                    kind: "ref".into(),
                });
                add_to_code_map(code_map, r, &cov.scenario.id);
            }
            for t in &cov.tests {
                let file_str = t.file.display().to_string();
                let deduped = !links.iter().any(|l| {
                    l.scenario == cov.scenario.id && l.file == file_str && l.kind == "test"
                });
                if deduped {
                    links.push(ExportLink {
                        scenario: cov.scenario.id.clone(),
                        file: file_str.clone(),
                        kind: "test".into(),
                    });
                    add_to_code_map(code_map, &file_str, &cov.scenario.id);
                }
            }
            ExportScenario {
                id: cov.scenario.id.clone(),
                name: cov.scenario.name.clone(),
                criticality: cov.scenario.criticality.as_str().into(),
                status: status_str(cov.status).into(),
                test_count: cov.tests.len(),
                refs: cov.scenario.refs.clone(),
                given: cov.scenario.given.clone(),
                when: cov.scenario.when.clone(),
                then: cov.scenario.then.clone(),
                wip: cov.scenario.wip,
                issue: cov.scenario.issue.clone(),
            }
        })
        .collect();

    ExportSpec {
        name: spec_name(&sc.file),
        module: sc.spec.module.clone(),
        scenarios,
    }
}

fn add_to_code_map(
    code_map: &mut HashMap<String, HashMap<String, Vec<String>>>,
    file_path: &str,
    scenario_id: &str,
) {
    let crate_name = extract_crate_name(file_path);
    let entry = code_map
        .entry(crate_name)
        .or_default()
        .entry(file_path.to_string())
        .or_default();
    if !entry.contains(&scenario_id.to_string()) {
        entry.push(scenario_id.to_string());
    }
}

fn extract_crate_name(path: &str) -> String {
    // Extract crate name from paths like "crates/arkavo-crypto/src/lib.rs"
    // and strip the redundant "arkavo-" prefix for display
    let parts: Vec<&str> = path.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        if *part == "crates"
            && let Some(name) = parts.get(i + 1)
        {
            return name.strip_prefix("arkavo-").unwrap_or(name).to_string();
        }
    }
    "other".into()
}

fn build_code_tree(code_map: HashMap<String, HashMap<String, Vec<String>>>) -> Vec<ExportCrate> {
    let mut crates: Vec<ExportCrate> = code_map
        .into_iter()
        .map(|(name, files)| {
            let mut file_list: Vec<ExportFile> = files
                .into_iter()
                .map(|(path, scenarios)| ExportFile { path, scenarios })
                .collect();
            file_list.sort_by(|a, b| a.path.cmp(&b.path));
            ExportCrate {
                name,
                files: file_list,
            }
        })
        .collect();
    crates.sort_by(|a, b| a.name.cmp(&b.name));
    crates
}
