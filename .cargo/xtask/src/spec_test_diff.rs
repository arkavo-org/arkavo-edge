use crate::spec_test_export::ExportData;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

pub fn load_export_data(path: &Path) -> Result<ExportData> {
    let json = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&json)?)
}

pub fn diff_markdown(baseline: &ExportData, current: &ExportData) -> String {
    let mut md = String::from("## Spec Coverage Delta\n\n");

    let b = &baseline.summary;
    let c = &current.summary;

    let pct_delta = c.pct - b.pct;
    let arrow = if pct_delta > 0.0 { "+" } else { "" };

    md.push_str("| Metric | main | PR | Delta |\n");
    md.push_str("|--------|------|----|-------|\n");
    md.push_str(&format!(
        "| Coverage | {:.1}% | {:.1}% | {arrow}{:.1}% |\n",
        b.pct, c.pct, pct_delta
    ));
    md.push_str(&format!(
        "| Scenarios | {} | {} | {} |\n",
        b.total,
        c.total,
        format_delta(c.total, b.total)
    ));
    md.push_str(&format!(
        "| Covered | {} | {} | {} |\n",
        b.covered,
        c.covered,
        format_delta(c.covered, b.covered)
    ));
    md.push_str(&format!(
        "| Partial | {} | {} | {} |\n",
        b.partial,
        c.partial,
        format_delta(c.partial, b.partial)
    ));
    md.push_str(&format!(
        "| Missing | {} | {} | {} |\n",
        b.missing,
        c.missing,
        format_delta(c.missing, b.missing)
    ));

    let base_scenarios = scenario_status_map(baseline);
    let curr_scenarios = scenario_status_map(current);

    let mut newly_covered: Vec<(&str, &str, &str)> = Vec::new();
    let mut regressions: Vec<(&str, &str, &str)> = Vec::new();
    let mut new_scenarios: Vec<&str> = Vec::new();
    let mut removed_scenarios: Vec<&str> = Vec::new();

    for (id, curr_status) in &curr_scenarios {
        match base_scenarios.get(*id) {
            Some(base_status) => {
                if *base_status != *curr_status {
                    if is_improvement(base_status, curr_status) {
                        newly_covered.push((id, base_status, curr_status));
                    } else {
                        regressions.push((id, base_status, curr_status));
                    }
                }
            }
            None => new_scenarios.push(id),
        }
    }

    for id in base_scenarios.keys() {
        if !curr_scenarios.contains_key(*id) {
            removed_scenarios.push(id);
        }
    }

    if !newly_covered.is_empty() {
        md.push_str("\n### Newly Covered\n\n");
        for (id, from, to) in &newly_covered {
            md.push_str(&format!("- `{id}`: {from} -> {to}\n"));
        }
    }

    if !regressions.is_empty() {
        md.push_str("\n### Regressions\n\n");
        for (id, from, to) in &regressions {
            md.push_str(&format!("- `{id}`: {from} -> {to}\n"));
        }
    }

    if !new_scenarios.is_empty() {
        md.push_str("\n### New Scenarios\n\n");
        for id in &new_scenarios {
            md.push_str(&format!("- `{id}`\n"));
        }
    }

    if !removed_scenarios.is_empty() {
        md.push_str("\n### Removed Scenarios\n\n");
        for id in &removed_scenarios {
            md.push_str(&format!("- `{id}`\n"));
        }
    }

    if newly_covered.is_empty()
        && regressions.is_empty()
        && new_scenarios.is_empty()
        && removed_scenarios.is_empty()
        && pct_delta.abs() < 0.05
    {
        md.push_str("\nNo scenario-level changes.\n");
    }

    md
}

fn format_delta(current: usize, baseline: usize) -> String {
    let diff = current as i64 - baseline as i64;
    if diff > 0 {
        format!("+{diff}")
    } else if diff < 0 {
        format!("{diff}")
    } else {
        "0".into()
    }
}

fn scenario_status_map(data: &ExportData) -> HashMap<&str, &str> {
    let mut map = HashMap::new();
    for spec in &data.specs {
        for sc in &spec.scenarios {
            map.insert(&*sc.id, &*sc.status);
        }
    }
    map
}

fn is_improvement(from: &str, to: &str) -> bool {
    let rank = |s: &str| match s {
        "covered" => 2,
        "partial" => 1,
        _ => 0,
    };
    rank(to) > rank(from)
}
