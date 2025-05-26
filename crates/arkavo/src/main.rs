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

    let command_args = args.get(1..).unwrap_or_default().to_vec();

    if let Err(err) = arkavo_cli::run(&command_args) {
        eprintln!("Error: {}", err);
        process::exit(1);
    }
}
