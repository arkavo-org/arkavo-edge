use std::env;
use std::io::IsTerminal;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();

    // Check if we need to relaunch in Terminal (macOS only)
    // Skip for serve command which needs to stay in current process
    #[cfg(target_os = "macos")]
    if args.get(1).is_none_or(|s| s != "serve") {
        maybe_relaunch_in_terminal();
    }

    // Handle version with git commit hash
    if args.len() > 1 && (args[1] == "--version" || args[1] == "-v") {
        println!(
            "arkavo {} ({})",
            env!("CARGO_PKG_VERSION"),
            env!("GIT_COMMIT_HASH")
        );
        return;
    }

    // If launched without arguments (e.g., via `open`), default to chat mode
    let command_args = if args.len() <= 1 {
        vec!["chat".to_string()]
    } else {
        args.get(1..).unwrap_or_default().to_vec()
    };

    if let Err(err) = arkavo_cli::run(&command_args) {
        eprintln!("Error: {err}");
        process::exit(1);
    }
}

#[cfg(target_os = "macos")]
fn maybe_relaunch_in_terminal() {
    use std::process::Command;

    // Check if we're already in a TTY
    if std::io::stdout().is_terminal() {
        return; // Already in terminal
    }

    // Check if we've already been relaunched
    if env::var_os("ARKAVO_LAUNCHED").is_some() {
        return; // Avoid infinite loop
    }

    // Get the path to our executable
    if let Ok(exe) = env::current_exe() {
        // Build command with arguments
        let args: Vec<String> = env::args().skip(1).collect();
        let arg_string = if args.is_empty() {
            "chat".to_string()
        } else {
            args.join(" ")
        };

        // Launch in Terminal.app using AppleScript with environment variable
        let script = format!(
            r#"tell application "Terminal"
                activate
                do script "ARKAVO_LAUNCHED=1 {} {}"
            end tell"#,
            exe.to_string_lossy(),
            arg_string
        );

        let mut cmd = Command::new("osascript");
        cmd.arg("-e").arg(script);

        // Spawn and exit
        let _ = cmd.spawn();
        process::exit(0);
    }
}
