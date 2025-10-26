use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();

    // Handle version with git commit hash
    if args.len() > 1 && (args[1] == "--version" || args[1] == "-v") {
        println!(
            "arkavo {} ({})",
            env!("CARGO_PKG_VERSION"),
            env!("GIT_COMMIT_HASH")
        );
        return;
    }

    // Detect if running from app bundle
    let from_app_bundle = env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.contains(".app/Contents/MacOS")))
        .unwrap_or(false);

    // If launched without arguments, default behavior depends on context
    let command_args = if args.len() <= 1 {
        if from_app_bundle {
            // From app bundle: launch UI
            vec!["ui".to_string()]
        } else {
            // From command line: default to agent run
            vec!["agent".to_string(), "run".to_string()]
        }
    } else {
        args.get(1..).unwrap_or_default().to_vec()
    };

    if let Err(err) = arkavo_cli::run(&command_args) {
        eprintln!("Error: {err}");
        process::exit(1);
    }
}
