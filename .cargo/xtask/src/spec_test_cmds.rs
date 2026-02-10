use crate::spec_test::{
    CoverageAnalyzer, CoverageStatus, Criticality, SpecParser, TestDiscovery, TestGenerator,
};
use anyhow::Result;
use clap::Subcommand;
use colored::Colorize;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum Commands {
    /// Show coverage summary
    Coverage {
        /// Show detailed per-spec coverage
        #[arg(short, long)]
        detailed: bool,
        /// Filter by spec name
        #[arg(long)]
        spec: Option<String>,
    },
    /// List uncovered scenarios
    Uncovered {
        /// Generate test stubs for uncovered scenarios
        #[arg(short, long)]
        generate: bool,
        /// Output directory for generated stubs
        #[arg(short, long, default_value = "tests/generated")]
        output: PathBuf,
    },
    /// Generate test stubs
    Generate {
        /// Spec name to generate for
        spec: Option<String>,
        /// Only generate for uncovered scenarios
        #[arg(short, long)]
        uncovered_only: bool,
        /// Output directory
        #[arg(short, long, default_value = "tests/generated")]
        output: PathBuf,
    },
    /// Run spec-tagged tests (shows cargo test command)
    Run {
        /// Run tests for specific scenario ID
        #[arg(long)]
        scenario: Option<String>,
        /// Run tests for specific spec
        #[arg(long)]
        spec: Option<String>,
        /// Filter by criticality
        #[arg(long)]
        criticality: Option<String>,
    },
    /// List all scenarios
    List {
        /// Filter by spec name
        #[arg(long)]
        spec: Option<String>,
        /// Show test count for each scenario
        #[arg(short, long)]
        with_tests: bool,
    },
}

pub fn run(command: Commands, specs_dir: PathBuf, crates_dir: PathBuf) -> Result<()> {
    match command {
        Commands::Coverage { detailed, spec } => {
            cmd_coverage(&specs_dir, &crates_dir, detailed, spec)
        }
        Commands::Uncovered { generate, output } => {
            cmd_uncovered(&specs_dir, &crates_dir, generate, output)
        }
        Commands::Generate {
            spec,
            uncovered_only,
            output,
        } => cmd_generate(&specs_dir, spec, uncovered_only, output),
        Commands::Run {
            scenario,
            spec,
            criticality,
        } => cmd_run(scenario, spec, criticality),
        Commands::List { spec, with_tests } => cmd_list(&specs_dir, &crates_dir, spec, with_tests),
    }
}

fn cmd_coverage(
    specs_dir: &PathBuf,
    crates_dir: &PathBuf,
    detailed: bool,
    filter_spec: Option<String>,
) -> Result<()> {
    println!("{}", "Spec Coverage Report".bold().cyan());
    println!("{}", "====================".cyan());
    println!();

    let specs = SpecParser::parse_all_specs(specs_dir)?;
    let tests = TestDiscovery::new()?.discover_tests(crates_dir)?;
    let report = CoverageAnalyzer::analyze(specs, tests);

    let specs: Vec<_> = if let Some(filter) = filter_spec {
        let filter_lower = filter.to_lowercase();
        report
            .specs
            .into_iter()
            .filter(|s| {
                let name = s
                    .file
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .replace(".spec", "")
                    .to_lowercase();
                name.contains(&filter_lower)
                    || s.spec.feature.to_lowercase().contains(&filter_lower)
            })
            .collect()
    } else {
        report.specs
    };

    println!(
        "{:<25} {:>8} {:>8} {:>12}",
        "Spec", "Total", "Covered", "Status"
    );
    println!("{}", "-".repeat(60));

    for spec_cov in &specs {
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

        let status = if covered == total {
            "🟢 Full".green()
        } else if covered > 0 || partial > 0 {
            "🟡 Partial".yellow()
        } else {
            "🔴 Missing".red()
        };

        let name = spec_cov
            .file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .replace(".spec", "");

        println!("{:<25} {:>8} {:>8} {:>12}", name, total, covered, status);
    }

    println!("{}", "-".repeat(60));
    let total_scenarios: usize = specs.iter().map(|s| s.scenarios.len()).sum();
    let total_covered: usize = specs
        .iter()
        .map(|s| {
            s.scenarios
                .iter()
                .filter(|sc| matches!(sc.status, CoverageStatus::Covered))
                .count()
        })
        .sum();

    println!(
        "{:<25} {:>8} {:>8}",
        "TOTAL", total_scenarios, total_covered
    );
    let pct = if total_scenarios > 0 {
        (total_covered as f64 / total_scenarios as f64) * 100.0
    } else {
        0.0
    };
    println!("\nCoverage: {pct:.1}%\n");

    if detailed {
        for spec_cov in &specs {
            println!("{} {}", "▶".cyan(), spec_cov.spec.feature.bold());
            for sc in &spec_cov.scenarios {
                println!(
                    "  {} {} - {} ({} tests)",
                    sc.status.emoji(),
                    sc.scenario.id.dimmed(),
                    sc.scenario.name,
                    sc.tests.len()
                );
            }
            println!();
        }
    }
    Ok(())
}

fn cmd_uncovered(
    specs_dir: &PathBuf,
    crates_dir: &PathBuf,
    generate: bool,
    output: PathBuf,
) -> Result<()> {
    println!("{}", "Uncovered Scenarios".bold().yellow());
    println!("{}\n", "==================".yellow());

    let specs = SpecParser::parse_all_specs(specs_dir)?;
    let tests = TestDiscovery::new()?.discover_tests(crates_dir)?;
    let report = CoverageAnalyzer::analyze(specs, tests);

    let uncovered_by_spec: Vec<_> = report
        .specs
        .iter()
        .map(|s| {
            let uncovered: Vec<_> = s
                .scenarios
                .iter()
                .filter(|sc| matches!(sc.status, CoverageStatus::Missing))
                .collect();
            (s, uncovered)
        })
        .filter(|(_, u)| !u.is_empty())
        .collect();

    if uncovered_by_spec.is_empty() {
        println!("{}", "All scenarios have test coverage!".green());
        return Ok(());
    }

    for (spec_cov, uncovered) in &uncovered_by_spec {
        let spec_name = spec_cov
            .file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        println!(
            "{} {} ({} uncovered)",
            "▶".red(),
            spec_name.bold(),
            uncovered.len()
        );
        for sc in uncovered {
            println!(
                "  {} {}: {}",
                "•".red(),
                sc.scenario.id.dimmed(),
                sc.scenario.name
            );
            println!(
                "    [{}] {}",
                sc.scenario.criticality.as_str().to_uppercase(),
                sc.scenario.when
            );
        }
        println!();
    }

    if generate {
        std::fs::create_dir_all(&output)?;
        for (spec_cov, _) in &uncovered_by_spec {
            let spec_name = spec_cov
                .file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .replace(".spec", "");
            let output_file = output.join(format!("{spec_name}_tests.rs"));
            let content = TestGenerator::generate_full_module(&spec_cov.spec, true, &[]);
            std::fs::write(&output_file, content)?;
            println!("Generated: {}", output_file.display());
        }
    }
    Ok(())
}

fn cmd_generate(
    specs_dir: &PathBuf,
    filter_spec: Option<String>,
    uncovered_only: bool,
    output: PathBuf,
) -> Result<()> {
    println!("{}\n", "Generating Test Stubs".bold().green());
    std::fs::create_dir_all(&output)?;

    let specs = SpecParser::parse_all_specs(specs_dir)?;
    for (path, spec) in specs {
        let spec_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .replace(".spec", "");
        if let Some(ref filter) = filter_spec {
            let filter_lower = filter.to_lowercase();
            if !spec_name.to_lowercase().contains(&filter_lower)
                && !spec.feature.to_lowercase().contains(&filter_lower)
            {
                continue;
            }
        }
        let output_file = output.join(format!("{spec_name}_tests.rs"));
        let content = TestGenerator::generate_full_module(&spec, uncovered_only, &[]);
        std::fs::write(&output_file, content)?;
        println!(
            "Generated: {} ({} scenarios)",
            output_file.display(),
            spec.scenarios.len()
        );
    }
    println!(
        "\n{} {}",
        "Done!".green(),
        format!("Stubs written to: {}", output.display()).dimmed()
    );
    Ok(())
}

fn cmd_run(
    scenario: Option<String>,
    spec: Option<String>,
    criticality: Option<String>,
) -> Result<()> {
    println!("{}\n", "Running Spec Tests".bold().blue());
    let filter = if let Some(ref s) = scenario {
        println!("Running tests for scenario: {}", s.yellow());
        Some(s.clone())
    } else if let Some(ref s) = spec {
        println!("Running tests for spec: {}", s.yellow());
        Some(s.clone())
    } else if let Some(ref c) = criticality {
        println!("Running tests with criticality: {}", c.yellow());
        Some(format!("criticality:{c}"))
    } else {
        println!("Running all spec-tagged tests");
        None
    };

    println!("\n{}", "Execute:".dimmed());
    match filter {
        Some(f) => println!("  cargo test {f}"),
        None => println!("  cargo test"),
    }
    Ok(())
}

fn cmd_list(
    specs_dir: &PathBuf,
    crates_dir: &PathBuf,
    filter_spec: Option<String>,
    with_tests: bool,
) -> Result<()> {
    println!("{}\n", "Spec Scenarios".bold().cyan());

    let specs = SpecParser::parse_all_specs(specs_dir)?;
    let tests = if with_tests {
        Some(TestDiscovery::new()?.discover_tests(crates_dir)?)
    } else {
        None
    };

    for (path, spec) in specs {
        let spec_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        if let Some(ref filter) = filter_spec {
            let filter_lower = filter.to_lowercase();
            if !spec_name.to_lowercase().contains(&filter_lower)
                && !spec.feature.to_lowercase().contains(&filter_lower)
            {
                continue;
            }
        }
        println!(
            "{} {} - {}",
            "▶".cyan(),
            spec_name.bold(),
            spec.feature.dimmed()
        );
        for scenario in &spec.scenarios {
            let crit_color = match scenario.criticality {
                Criticality::Critical => "critical".red(),
                Criticality::High => "high".yellow(),
                _ => scenario.criticality.as_str().normal(),
            };
            print!(
                "  {} {} [{}] {}",
                "•".normal(),
                scenario.id.dimmed(),
                crit_color,
                scenario.name
            );
            if let Some(ref t) = tests {
                let count = t
                    .iter()
                    .filter(|test| test.scenarios_covered.contains(&scenario.id))
                    .count();
                if count > 0 {
                    print!(" {}", format!("({count} tests)").green());
                } else {
                    print!(" {}", "(no tests)".red());
                }
            }
            println!();
        }
        println!();
    }
    Ok(())
}
