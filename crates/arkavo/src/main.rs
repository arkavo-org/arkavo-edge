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

    // Check if we need to relaunch in Terminal (macOS only)
    // Skip for serve, agent, and ui commands which need to stay in current process
    // Also skip if --prompt is provided (for testing/CI) or from app bundle
    #[cfg(target_os = "macos")]
    if !from_app_bundle
        && command_args.first().is_some_and(|s| s != "serve" && s != "agent" && s != "ui")
        && !args.iter().any(|arg| arg == "--prompt")
    {
        maybe_relaunch_in_terminal(&command_args);
    }

    if let Err(err) = arkavo_cli::run(&command_args) {
        eprintln!("Error: {err}");
        process::exit(1);
    }
}

#[cfg(target_os = "macos")]
fn maybe_relaunch_in_terminal(command_args: &[String]) {
    use std::io::IsTerminal;
    use std::process::Command;

    // Check if terminal relaunch is disabled via environment variable (for testing/automation)
    if env::var("ARKAVO_NO_TERMINAL_RELAUNCH").is_ok() {
        return; // Skip terminal relaunch for automation/testing
    }

    // Check if we're already in a TTY
    if std::io::stdout().is_terminal() {
        return; // Already in terminal
    }

    // Get the path to our executable
    if let Ok(exe) = env::current_exe() {
        // Build command with arguments
        let arg_string = command_args.join(" ");

        // Launch in Terminal.app using AppleScript
        let script = format!(
            r#"tell application "Terminal"
                activate
                do script "{} {}"
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
