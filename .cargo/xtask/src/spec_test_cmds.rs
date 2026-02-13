use crate::spec_test::{
    CoverageAnalyzer, CoverageStatus, Criticality, SpecCoverage, SpecParser, TestDiscovery,
    TestGenerator,
};
use crate::spec_test_report;
use anyhow::Result;
use clap::Subcommand;
use colored::Colorize;
use std::path::{Path, PathBuf};
use std::process;

#[derive(Subcommand)]
pub enum Commands {
    /// Show coverage summary
    Coverage {
        #[arg(short, long)]
        detailed: bool,
        #[arg(long)]
        spec: Option<String>,
        /// Output GitHub-flavored markdown (for CI PR comments)
        #[arg(long)]
        markdown: bool,
        /// Exit non-zero if coverage percentage is below this threshold
        #[arg(long)]
        fail_under: Option<f64>,
        /// Exit non-zero if any critical/high scenario has zero tests
        #[arg(long)]
        critical_required: bool,
    },
    /// List uncovered scenarios
    Uncovered {
        #[arg(short, long)]
        generate: bool,
        #[arg(short, long, default_value = "tests/generated")]
        output: PathBuf,
    },
    /// Generate test stubs
    Generate {
        spec: Option<String>,
        #[arg(short, long)]
        uncovered_only: bool,
        #[arg(short, long, default_value = "tests/generated")]
        output: PathBuf,
    },
    /// Run spec-tagged tests (shows cargo test command)
    Run {
        #[arg(long)]
        scenario: Option<String>,
        #[arg(long)]
        spec: Option<String>,
        #[arg(long)]
        criticality: Option<String>,
    },
    /// List all scenarios
    List {
        #[arg(long)]
        spec: Option<String>,
        #[arg(short, long)]
        with_tests: bool,
    },
}

pub fn run(command: Commands, specs_dir: PathBuf, crates_dir: PathBuf) -> Result<()> {
    match command {
        Commands::Coverage {
            detailed, spec, markdown, fail_under, critical_required,
        } => cmd_coverage(&specs_dir, &crates_dir, detailed, spec, markdown, fail_under, critical_required),
        Commands::Uncovered { generate, output } => {
            cmd_uncovered(&specs_dir, &crates_dir, generate, output)
        }
        Commands::Generate { spec, uncovered_only, output } => {
            cmd_generate(&specs_dir, spec, uncovered_only, output)
        }
        Commands::Run { scenario, spec, criticality } => cmd_run(scenario, spec, criticality),
        Commands::List { spec, with_tests } => cmd_list(&specs_dir, &crates_dir, spec, with_tests),
    }
}

fn spec_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .replace(".spec", "")
}

fn filter_specs(specs: Vec<SpecCoverage>, filter: Option<String>) -> Vec<SpecCoverage> {
    match filter {
        Some(f) => {
            let f = f.to_lowercase();
            specs
                .into_iter()
                .filter(|s| {
                    spec_name(&s.file).to_lowercase().contains(&f)
                        || s.spec.feature.to_lowercase().contains(&f)
                })
                .collect()
        }
        None => specs,
    }
}

fn cmd_coverage(
    specs_dir: &Path, crates_dir: &Path, detailed: bool, filter_spec: Option<String>,
    markdown: bool, fail_under: Option<f64>, critical_required: bool,
) -> Result<()> {
    let specs = SpecParser::parse_all_specs(specs_dir)?;
    let tests = TestDiscovery::new()?.discover_tests(crates_dir)?;
    let report = CoverageAnalyzer::analyze(specs, tests);
    let pct = report.coverage_percentage();
    let (total, covered) = (report.total_scenarios, report.covered_scenarios);
    let filtered = filter_specs(report.specs, filter_spec);

    if markdown {
        let (md, gate) = spec_test_report::format_markdown_report(
            &filtered, total, covered, pct, fail_under, critical_required,
        );
        print!("{md}");
        if !gate.passed {
            for msg in &gate.messages {
                eprintln!("QUALITY_GATE_FAILED: {msg}");
            }
            process::exit(1);
        }
        return Ok(());
    }

    print_terminal_report(&filtered, detailed, pct);
    Ok(())
}

fn count_by_status(spec: &SpecCoverage, status: CoverageStatus) -> usize {
    spec.scenarios.iter().filter(|s| s.status == status).count()
}

fn print_terminal_report(specs: &[SpecCoverage], detailed: bool, pct: f64) {
    println!("{}\n{}\n", "Spec Coverage Report".bold().cyan(), "====================".cyan());
    println!("{:<25} {:>8} {:>8} {:>12}", "Spec", "Total", "Covered", "Status");
    println!("{}", "-".repeat(60));

    for s in specs {
        let (total, covered) = (s.scenarios.len(), count_by_status(s, CoverageStatus::Covered));
        let partial = count_by_status(s, CoverageStatus::Partial);
        let status = if covered == total {
            "Full".green()
        } else if covered > 0 || partial > 0 {
            "Partial".yellow()
        } else {
            "Missing".red()
        };
        println!("{:<25} {:>8} {:>8} {:>12}", spec_name(&s.file), total, covered, status);
    }

    println!("{}", "-".repeat(60));
    let total: usize = specs.iter().map(|s| s.scenarios.len()).sum();
    let covered: usize = specs.iter().map(|s| count_by_status(s, CoverageStatus::Covered)).sum();
    println!("{:<25} {:>8} {:>8}", "TOTAL", total, covered);
    println!("\nCoverage: {pct:.1}%\n");

    if detailed {
        for s in specs {
            println!("{} {}", "▶".cyan(), s.spec.feature.bold());
            for sc in &s.scenarios {
                println!(
                    "  {} {} - {} ({} tests)",
                    sc.status.emoji(), sc.scenario.id.dimmed(), sc.scenario.name, sc.tests.len()
                );
            }
            println!();
        }
    }
}

fn cmd_uncovered(
    specs_dir: &Path, crates_dir: &Path, generate: bool, output: PathBuf,
) -> Result<()> {
    println!("{}\n{}\n", "Uncovered Scenarios".bold().yellow(), "==================".yellow());

    let specs = SpecParser::parse_all_specs(specs_dir)?;
    let tests = TestDiscovery::new()?.discover_tests(crates_dir)?;
    let report = CoverageAnalyzer::analyze(specs, tests);

    let uncovered_by_spec: Vec<_> = report
        .specs.iter()
        .map(|s| {
            let uncovered: Vec<_> = s.scenarios.iter()
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
        let name = spec_name(&spec_cov.file);
        println!("{} {} ({} uncovered)", "▶".red(), name.bold(), uncovered.len());
        for sc in uncovered {
            println!("  {} {}: {}", "•".red(), sc.scenario.id.dimmed(), sc.scenario.name);
            println!(
                "    [{}] {}",
                sc.scenario.criticality.as_str().to_uppercase(), sc.scenario.when
            );
        }
        println!();
    }

    if generate {
        std::fs::create_dir_all(&output)?;
        for (spec_cov, _) in &uncovered_by_spec {
            let name = spec_name(&spec_cov.file);
            let output_file = output.join(format!("{name}_tests.rs"));
            let content = TestGenerator::generate_full_module(&spec_cov.spec, true, &[]);
            std::fs::write(&output_file, content)?;
            println!("Generated: {}", output_file.display());
        }
    }
    Ok(())
}

fn cmd_generate(
    specs_dir: &Path, filter_spec: Option<String>, uncovered_only: bool, output: PathBuf,
) -> Result<()> {
    println!("{}\n", "Generating Test Stubs".bold().green());
    std::fs::create_dir_all(&output)?;

    let specs = SpecParser::parse_all_specs(specs_dir)?;
    for (path, spec) in specs {
        let name = spec_name(&path);
        if let Some(ref filter) = filter_spec {
            let f = filter.to_lowercase();
            if !name.to_lowercase().contains(&f) && !spec.feature.to_lowercase().contains(&f) {
                continue;
            }
        }
        let output_file = output.join(format!("{name}_tests.rs"));
        let content = TestGenerator::generate_full_module(&spec, uncovered_only, &[]);
        std::fs::write(&output_file, content)?;
        println!("Generated: {} ({} scenarios)", output_file.display(), spec.scenarios.len());
    }
    println!(
        "\n{} {}",
        "Done!".green(),
        format!("Stubs written to: {}", output.display()).dimmed()
    );
    Ok(())
}

fn cmd_run(
    scenario: Option<String>, spec: Option<String>, criticality: Option<String>,
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
    specs_dir: &Path, crates_dir: &Path, filter_spec: Option<String>, with_tests: bool,
) -> Result<()> {
    println!("{}\n", "Spec Scenarios".bold().cyan());

    let specs = SpecParser::parse_all_specs(specs_dir)?;
    let tests = if with_tests {
        Some(TestDiscovery::new()?.discover_tests(crates_dir)?)
    } else {
        None
    };

    for (path, spec) in specs {
        let name = spec_name(&path);
        if let Some(ref filter) = filter_spec {
            let f = filter.to_lowercase();
            if !name.to_lowercase().contains(&f) && !spec.feature.to_lowercase().contains(&f) {
                continue;
            }
        }
        println!("{} {} - {}", "▶".cyan(), name.bold(), spec.feature.dimmed());
        for scenario in &spec.scenarios {
            let crit_color = match scenario.criticality {
                Criticality::Critical => "critical".red(),
                Criticality::High => "high".yellow(),
                _ => scenario.criticality.as_str().normal(),
            };
            print!("  {} {} [{}] {}", "•".normal(), scenario.id.dimmed(), crit_color, scenario.name);
            if let Some(ref t) = tests {
                let count = t.iter()
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
