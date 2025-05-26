use arkavo_test::gherkin::parser::Parser;
use arkavo_test::execution::runner::TestRunner;
use arkavo_test::reporting::business_report::{BusinessReporter, OutputFormat};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let feature_path = args.first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tests"));
    
    if feature_path.is_file() && feature_path.extension() == Some(std::ffi::OsStr::new("feature")) {
        run_gherkin_test(&feature_path)
    } else {
        run_project_tests(&feature_path)
    }
}

fn run_gherkin_test(feature_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("Running Gherkin test: {}", feature_path.display());
    
    let feature = Parser::parse_feature_file(feature_path)?;
    
    println!("Feature: {}", feature.name);
    if let Some(desc) = &feature.description {
        println!("Description: {}", desc);
    }
    
    let runtime = tokio::runtime::Runtime::new()?;
    let runner = TestRunner::new();
    
    let results = runtime.block_on(async {
        runner.run_parallel_scenarios(feature.scenarios).await
    })?;
    
    let reporter = BusinessReporter::new(OutputFormat::Markdown)?;
    let report = reporter.generate_report(&results)?;
    
    println!("\n{}", report);
    
    let failed = results.iter().any(|r| r.status == arkavo_test::reporting::business_report::TestStatus::Failed);
    if failed {
        std::process::exit(1);
    }
    
    Ok(())
}

fn run_project_tests(test_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let project_type = detect_project_type()?;
    
    match project_type {
        ProjectType::Rust => run_rust_tests(test_path),
        ProjectType::Python => run_python_tests(test_path),
        ProjectType::JavaScript => run_javascript_tests(test_path),
        ProjectType::Go => run_go_tests(test_path),
        ProjectType::Unknown => {
            eprintln!("Could not detect project type. Supported: Rust, Python, JavaScript, Go");
            std::process::exit(1);
        }
    }
}

#[derive(Debug)]
enum ProjectType {
    Rust,
    Python,
    JavaScript,
    Go,
    Unknown,
}

fn detect_project_type() -> Result<ProjectType, Box<dyn std::error::Error>> {
    if Path::new("Cargo.toml").exists() {
        Ok(ProjectType::Rust)
    } else if Path::new("setup.py").exists() || Path::new("pyproject.toml").exists() {
        Ok(ProjectType::Python)
    } else if Path::new("package.json").exists() {
        Ok(ProjectType::JavaScript)
    } else if Path::new("go.mod").exists() {
        Ok(ProjectType::Go)
    } else {
        Ok(ProjectType::Unknown)
    }
}

fn run_rust_tests(_test_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("Running Rust tests...");
    let output = Command::new("cargo")
        .arg("test")
        .arg("--color=always")
        .output()?;
    
    print!("{}", String::from_utf8_lossy(&output.stdout));
    print!("{}", String::from_utf8_lossy(&output.stderr));
    
    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }
    
    Ok(())
}

fn run_python_tests(test_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("Running Python tests...");
    let mut cmd = Command::new("python");
    cmd.arg("-m").arg("pytest");
    
    if test_path != Path::new("tests") {
        cmd.arg(test_path);
    }
    
    let output = cmd.output()?;
    
    print!("{}", String::from_utf8_lossy(&output.stdout));
    print!("{}", String::from_utf8_lossy(&output.stderr));
    
    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }
    
    Ok(())
}

fn run_javascript_tests(_test_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("Running JavaScript tests...");
    let output = Command::new("npm")
        .arg("test")
        .output()?;
    
    print!("{}", String::from_utf8_lossy(&output.stdout));
    print!("{}", String::from_utf8_lossy(&output.stderr));
    
    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }
    
    Ok(())
}

fn run_go_tests(test_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("Running Go tests...");
    let mut cmd = Command::new("go");
    cmd.arg("test");
    
    if test_path == Path::new("tests") {
        cmd.arg("./...");
    } else {
        cmd.arg(test_path);
    }
    
    let output = cmd.output()?;
    
    print!("{}", String::from_utf8_lossy(&output.stdout));
    print!("{}", String::from_utf8_lossy(&output.stderr));
    
    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }
    
    Ok(())
}