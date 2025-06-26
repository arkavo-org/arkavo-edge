use std::env;
use std::process;

fn main() {
    // Check if we need to relaunch in Terminal (macOS only)
    #[cfg(target_os = "macos")]
    maybe_relaunch_in_terminal();

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

#[cfg(target_os = "macos")]
fn maybe_relaunch_in_terminal() {
    use std::process::Command;

    // Check if we're already in a TTY
    if atty::is(atty::Stream::Stdout) {
        return; // Already in terminal
    }

    // Check if we've already been relaunched
    if env::var_os("ARKAVO_LAUNCHED").is_some() {
        return; // Avoid infinite loop
    }

    // Get the path to our executable
    if let Ok(exe) = env::current_exe() {
        // Launch in Terminal.app
        let mut cmd = Command::new("open");
        cmd.arg("-a")
            .arg("Terminal")
            .arg(&exe)
            .env("ARKAVO_LAUNCHED", "1");

        // Pass through all arguments
        let args: Vec<String> = env::args().skip(1).collect();
        if !args.is_empty() {
            cmd.args(&args);
        }

        // Spawn and exit
        let _ = cmd.spawn();
        process::exit(0);
    }
}
