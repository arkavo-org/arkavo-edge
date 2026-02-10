use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
struct SpecIndex {
    components: Vec<Component>,
    stats: Stats,
}

#[derive(Deserialize)]
struct Component {
    name: String,
    file: String,
    module: String,
    description: String,
    criticality: String,
    scenario_count: u32,
    #[serde(default)]
    invariants: Vec<serde_yaml::Value>,
}

#[derive(Deserialize)]
struct Stats {
    total_specs: u32,
    total_scenarios: u32,
}

pub fn run(matrix: bool, list: bool, search: Option<String>, name: Option<String>) -> Result<()> {
    let index = load_index()?;

    if matrix {
        return print_matrix(&index);
    }
    if list {
        return print_list(&index);
    }
    if let Some(term) = search {
        return search_capabilities(&index, &term);
    }
    if let Some(n) = name {
        return show_detail(&index, &n);
    }

    interactive_mode(&index)
}

fn find_index_path() -> Result<PathBuf> {
    let relative = "specs/arkavo-edge/index.yaml";

    // Try relative to cwd (typical: run from repo root)
    let cwd = std::env::current_dir()?;
    let path = cwd.join(relative);
    if path.exists() {
        return Ok(path);
    }

    // Try relative to binary location
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent().and_then(|p| p.parent())
    {
        let path = parent.join(relative);
        if path.exists() {
            return Ok(path);
        }
    }

    anyhow::bail!("Could not find {relative}. Run from the repository root.")
}

fn load_index() -> Result<SpecIndex> {
    let path = find_index_path()?;
    let contents =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let index: SpecIndex = serde_yaml::from_str(&contents).context("parsing index.yaml")?;
    Ok(index)
}

fn interactive_mode(index: &SpecIndex) -> Result<()> {
    println!(
        "Arkavo Capabilities ({} specs, {} scenarios)\n",
        index.stats.total_specs, index.stats.total_scenarios
    );

    for (i, comp) in index.components.iter().enumerate() {
        let crit = criticality_icon(&comp.criticality);
        println!(
            "  {:>2}. {crit} {:<25} {}",
            i + 1,
            comp.name,
            comp.description
        );
    }

    println!();
    println!("Select a number for details, or use:");
    println!("  cargo xtask capabilities --matrix     Full table view");
    println!("  cargo xtask capabilities --list       Compact list");
    println!("  cargo xtask capabilities --search X   Filter by keyword");
    println!("  cargo xtask capabilities --name X     Show one spec");

    use std::io::{self, BufRead};
    print!("\n> ");
    io::Write::flush(&mut io::stdout())?;

    let stdin = io::stdin();
    let next_line = stdin.lock().lines().next();
    if let Some(Ok(line)) = next_line {
        let line = line.trim().to_string();
        if line.is_empty() {
            return Ok(());
        }
        if let Ok(num) = line.parse::<usize>() {
            if num >= 1 && num <= index.components.len() {
                return print_component_detail(&index.components[num - 1]);
            }
            eprintln!("Invalid selection: {num}");
        } else {
            return show_detail(index, &line);
        }
    }
    Ok(())
}

fn print_matrix(index: &SpecIndex) -> Result<()> {
    println!(
        "Arkavo Capabilities ({} specs, {} scenarios)\n",
        index.stats.total_specs, index.stats.total_scenarios
    );

    println!(
        "{:<25} {:>10} {:>10}   DESCRIPTION",
        "NAME", "CRITICALITY", "SCENARIOS"
    );
    println!("{}", "-".repeat(90));

    for comp in &index.components {
        println!(
            "{:<25} {:>10} {:>10}   {}",
            comp.name, comp.criticality, comp.scenario_count, comp.description
        );
    }
    Ok(())
}

fn print_list(index: &SpecIndex) -> Result<()> {
    for comp in &index.components {
        let crit = criticality_icon(&comp.criticality);
        println!("{crit} {:<25} {}", comp.name, comp.description);
    }
    println!(
        "\n{} specs, {} scenarios total",
        index.stats.total_specs, index.stats.total_scenarios
    );
    Ok(())
}

fn search_capabilities(index: &SpecIndex, term: &str) -> Result<()> {
    let term_lower = term.to_lowercase();
    let matches: Vec<&Component> = index
        .components
        .iter()
        .filter(|c| {
            c.name.to_lowercase().contains(&term_lower)
                || c.description.to_lowercase().contains(&term_lower)
                || c.module.to_lowercase().contains(&term_lower)
        })
        .collect();

    if matches.is_empty() {
        println!("No capabilities matching '{term}'");
        return Ok(());
    }

    println!("Found {} matching '{term}':\n", matches.len());
    for comp in matches {
        let crit = criticality_icon(&comp.criticality);
        println!("  {crit} {:<25} {}", comp.name, comp.description);
    }
    Ok(())
}

fn show_detail(index: &SpecIndex, name: &str) -> Result<()> {
    let name_lower = name.to_lowercase();
    let comp = index
        .components
        .iter()
        .find(|c| c.name.to_lowercase() == name_lower);

    match comp {
        Some(c) => print_component_detail(c),
        None => {
            anyhow::bail!("Unknown capability: '{name}'. Use --list to see all capabilities.")
        }
    }
}

fn print_component_detail(comp: &Component) -> Result<()> {
    let crit = criticality_icon(&comp.criticality);
    println!("{crit} {}", comp.name);
    println!();
    println!("  Description:  {}", comp.description);
    println!("  Module:       {}", comp.module);
    println!("  Criticality:  {}", comp.criticality);
    println!("  Scenarios:    {}", comp.scenario_count);
    println!("  Spec file:    specs/arkavo-edge/{}", comp.file);

    if !comp.invariants.is_empty() {
        println!();
        println!("  Invariants:");
        for inv in &comp.invariants {
            println!("    - {}", format_invariant(inv));
        }
    }

    println!();
    println!("  View spec:    cat specs/arkavo-edge/{}", comp.file);
    Ok(())
}

fn format_invariant(val: &serde_yaml::Value) -> String {
    match val {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Mapping(m) => {
            // YAML parsed "Key: value" as a mapping — rejoin as "Key: value"
            m.iter()
                .map(|(k, v)| {
                    let key = k.as_str().unwrap_or("");
                    let val = v.as_str().unwrap_or("");
                    format!("{key}: {val}")
                })
                .collect::<Vec<_>>()
                .join(", ")
        }
        other => format!("{other:?}"),
    }
}

fn criticality_icon(criticality: &str) -> &'static str {
    match criticality {
        "critical" => "[!!]",
        "high" => "[! ]",
        "medium" => "[  ]",
        _ => "[  ]",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_criticality_icon() {
        assert_eq!(criticality_icon("critical"), "[!!]");
        assert_eq!(criticality_icon("high"), "[! ]");
        assert_eq!(criticality_icon("medium"), "[  ]");
        assert_eq!(criticality_icon("low"), "[  ]");
    }

    #[test]
    fn test_deserialize_component() {
        let yaml = r#"
components:
  - name: test-comp
    file: test.spec.yaml
    module: test_module
    description: A test component
    criticality: high
    scenario_count: 5
    invariants:
      - First invariant
stats:
  total_specs: 1
  total_scenarios: 5
"#;
        let index: SpecIndex = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(index.components.len(), 1);
        assert_eq!(index.components[0].name, "test-comp");
        assert_eq!(index.components[0].scenario_count, 5);
        assert_eq!(index.stats.total_specs, 1);
    }

    #[test]
    fn test_format_invariant_string_and_mapping() {
        let s = serde_yaml::Value::String("Simple invariant".into());
        assert_eq!(format_invariant(&s), "Simple invariant");

        // YAML "Ed25519: 32-byte keys" parses as a mapping
        let yaml = "- Ed25519: 32-byte keys";
        let vals: Vec<serde_yaml::Value> = serde_yaml::from_str(yaml).unwrap();
        let formatted = format_invariant(&vals[0]);
        assert!(formatted.contains("Ed25519"));
        assert!(formatted.contains("32-byte keys"));
    }

    #[test]
    fn test_deserialize_no_invariants() {
        let yaml = r#"
components:
  - name: minimal
    file: minimal.spec.yaml
    module: minimal_mod
    description: Minimal component
    criticality: medium
    scenario_count: 3
stats:
  total_specs: 1
  total_scenarios: 3
"#;
        let index: SpecIndex = serde_yaml::from_str(yaml).unwrap();
        assert!(index.components[0].invariants.is_empty());
    }

    #[test]
    fn test_search_matches_name_description_module() {
        let yaml = r#"
components:
  - name: router
    file: router.spec.yaml
    module: arkavo_router
    description: LLM routing with quality gates
    criticality: critical
    scenario_count: 17
  - name: tdf
    file: tdf.spec.yaml
    module: arkavo_tdf
    description: Trusted Data Format encryption
    criticality: critical
    scenario_count: 9
stats:
  total_specs: 2
  total_scenarios: 26
"#;
        let index: SpecIndex = serde_yaml::from_str(yaml).unwrap();

        let term = "router";
        let matches: Vec<&Component> = index
            .components
            .iter()
            .filter(|c| {
                c.name.contains(term)
                    || c.description.to_lowercase().contains(term)
                    || c.module.to_lowercase().contains(term)
            })
            .collect();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "router");

        let term = "encryption";
        let matches: Vec<&Component> = index
            .components
            .iter()
            .filter(|c| {
                c.name.contains(term)
                    || c.description.to_lowercase().contains(term)
                    || c.module.to_lowercase().contains(term)
            })
            .collect();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "tdf");
    }
}
