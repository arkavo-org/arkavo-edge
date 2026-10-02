use arkavo_process_env::ChildEnv;
use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

/// Start an MCP server over stdio with exactly `env` as its environment.
///
/// Every stdio MCP server this crate starts goes through here, so none of
/// them inherits the agent's environment and the provider keys in it.
pub fn spawn_stdio(command: &str, args: &[String], env: &ChildEnv) -> std::io::Result<Child> {
    env.command(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

/// Manages spawning and lifecycle of MCP server processes
#[derive(Debug)]
pub struct McpProcessManager {
    processes: Arc<Mutex<Vec<TrackedProcess>>>,
}

#[derive(Debug)]
struct TrackedProcess {
    name: String,
    pid: u32,
}

#[derive(Debug)]
pub struct McpProcess {
    pub name: String,
    pub child: Child,
    pub stdin: ChildStdin,
    pub stdout: BufReader<ChildStdout>,
}

impl Default for McpProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

impl McpProcessManager {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Register an externally spawned process for tracking
    pub fn register_process(&self, name: String, pid: u32) {
        if let Ok(mut processes) = self.processes.lock() {
            processes.push(TrackedProcess { name, pid });
        }
    }

    /// Spawn a new MCP server process
    #[allow(clippy::missing_panics_doc)]
    pub fn spawn_mcp_server(
        &self,
        name: String,
        command: &str,
        args: &[String],
        env: &ChildEnv,
    ) -> Result<McpProcess, Box<dyn std::error::Error>> {
        // Validate that the command exists
        validate_command(command)?;

        // Debug log for MCP server spawn
        if std::env::var("ARKAVO_DEBUG").is_ok() {
            eprintln!("[DEBUG] mcp.spawner Spawning MCP server '{name}': {command} {args:?}");
        }

        // Start the process
        let mut child = spawn_stdio(command, args, env)
            .map_err(|e| format!("Failed to spawn MCP server '{command}': {e}"))?;

        // Take ownership of stdin and stdout
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("Failed to get stdin for MCP server '{name}'"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| format!("Failed to get stdout for MCP server '{name}'"))?;

        let stdout_reader = BufReader::new(stdout);

        // Optionally spawn a thread to capture stderr for debugging
        if let Some(stderr) = child.stderr.take() {
            let name_clone = name.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    // Only show stderr in debug mode
                    if std::env::var("ARKAVO_DEBUG").is_ok() {
                        eprintln!("[DEBUG] mcp.server.stderr name={name_clone} line=\"{line}\"");
                    }
                }
            });
        }

        // Get the process ID for tracking
        let pid = child.id();

        let process = McpProcess {
            name: name.clone(),
            child,
            stdin,
            stdout: stdout_reader,
        };

        // Track the process by PID
        if let Ok(mut processes) = self.processes.lock() {
            processes.push(TrackedProcess { name, pid });
        }

        Ok(process)
    }

    /// Shutdown all managed processes gracefully with timeout
    pub fn shutdown_all(&self) -> Result<(), Box<dyn std::error::Error>> {
        use std::thread;
        use std::time::Duration;

        let mut processes = self.processes.lock().unwrap_or_else(|e| e.into_inner());

        for tracked in processes.iter() {
            // Telemetry: MCP server shutting down
            println!(
                "[INFO] mcp.server.shutdown name={} pid={}",
                tracked.name, tracked.pid
            );
            eprintln!(
                "Shutting down MCP server '{}' (PID: {})",
                tracked.name, tracked.pid
            );

            // Use platform-specific process termination
            // Try graceful shutdown first (SIGTERM on Unix, similar on Windows)
            #[cfg(unix)]
            {
                match std::process::Command::new("kill")
                    .arg("-TERM")
                    .arg(tracked.pid.to_string())
                    .output()
                {
                    Ok(output) => {
                        if output.status.success() {
                            // Give process 5 seconds to shut down gracefully
                            let start = std::time::Instant::now();
                            while start.elapsed() < Duration::from_secs(5) {
                                // Check if process still exists
                                match std::process::Command::new("kill")
                                    .arg("-0") // Check if process exists
                                    .arg(tracked.pid.to_string())
                                    .output()
                                {
                                    Ok(check) => {
                                        if !check.status.success() {
                                            // Process has exited
                                            println!(
                                                "[INFO] mcp.server.exit name={} pid={} code=0",
                                                tracked.name, tracked.pid
                                            );
                                            break;
                                        }
                                    }
                                    Err(_) => {
                                        println!(
                                            "[INFO] mcp.server.exit name={} pid={} code=unknown",
                                            tracked.name, tracked.pid
                                        );
                                        break;
                                    }
                                }
                                thread::sleep(Duration::from_millis(100));
                            }

                            // If still running after 5 seconds, force kill
                            if let Ok(check) = std::process::Command::new("kill")
                                .arg("-0")
                                .arg(tracked.pid.to_string())
                                .output()
                                && check.status.success()
                            {
                                eprintln!(
                                    "MCP server '{}' (PID: {}) did not shut down gracefully, forcing termination",
                                    tracked.name, tracked.pid
                                );
                                let _ = std::process::Command::new("kill")
                                    .arg("-KILL")
                                    .arg(tracked.pid.to_string())
                                    .output();
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "Failed to send SIGTERM to MCP server '{}' (PID: {}): {}",
                            tracked.name, tracked.pid, e
                        );
                    }
                }
            }

            #[cfg(windows)]
            {
                // On Windows, use taskkill
                match std::process::Command::new("taskkill")
                    .arg("/PID")
                    .arg(tracked.pid.to_string())
                    .arg("/F") // Force termination
                    .output()
                {
                    Ok(output) => {
                        if !output.status.success() {
                            eprintln!(
                                "Failed to terminate MCP server '{}' (PID: {})",
                                tracked.name, tracked.pid
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "Failed to kill MCP server '{}' (PID: {}): {}",
                            tracked.name, tracked.pid, e
                        );
                    }
                }
            }
        }

        processes.clear();
        drop(processes);
        Ok(())
    }
}

impl Drop for McpProcessManager {
    fn drop(&mut self) {
        let _ = self.shutdown_all();
    }
}

/// Validate that a command exists and is executable
fn validate_command(command: &str) -> Result<(), Box<dyn std::error::Error>> {
    // For any path (relative like ./, ../ or absolute like /usr/bin/), check if file exists
    if command.contains('/') {
        if std::path::Path::new(command).exists() {
            return Ok(());
        }
        return Err(format!("Command not found at path: {command}").into());
    }

    // Use 'which' command on Unix-like systems to check if command exists
    #[cfg(unix)]
    {
        let output = Command::new("which")
            .arg(command)
            .output()
            .map_err(|e| format!("Failed to run 'which' command: {e}"))?;

        if !output.status.success() {
            return Err(format!(
                "Command '{command}' not found in PATH. Please ensure it is installed and accessible."
            )
            .into());
        }
    }

    // On Windows, use 'where' command
    #[cfg(windows)]
    {
        let output = Command::new("where")
            .arg(command)
            .output()
            .map_err(|e| format!("Failed to run 'where' command: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "Command '{}' not found in PATH. Please ensure it is installed and accessible.",
                command
            )
            .into());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_process_env::EnvSpec;
    use arkavo_test_macros::spec;
    use std::ffi::OsString;
    use std::io::Read;

    /// Starts `env` (Unix) or `set` (Windows) the way an MCP server is
    /// started and returns the `NAME=value` lines it printed.
    fn server_sees(env: &ChildEnv) -> Vec<String> {
        #[cfg(unix)]
        let (command, args) = ("sh", ["-c".to_string(), "env".to_string()]);
        #[cfg(windows)]
        let (command, args) = ("cmd", ["/C".to_string(), "set".to_string()]);
        let mut child = spawn_stdio(command, &args, env).expect("spawn environment dump");
        drop(child.stdin.take());
        let mut out = String::new();
        child
            .stdout
            .take()
            .expect("stdout")
            .read_to_string(&mut out)
            .expect("read environment dump");
        child.wait().expect("environment dump exits");
        out.lines()
            .map(|line| line.trim_end_matches('\r').to_string())
            .collect()
    }

    #[spec("MCPR-008")]
    #[test]
    fn mcp_server_sees_only_baseline_and_declared_environment() {
        let spec = EnvSpec {
            set: [("MCP_PROBE_CONFIGURED".to_string(), "yes".to_string())].into(),
            passthrough: vec!["GITHUB_TOKEN".to_string()],
        };
        let parent = [
            ("PATH", std::env::var_os("PATH").unwrap_or_default()),
            ("OPENAI_API_KEY", OsString::from("planted-secret")),
            ("GITHUB_TOKEN", OsString::from("declared-token")),
        ];
        let seen = server_sees(&ChildEnv::isolated(parent, &spec));

        assert!(
            seen.iter().any(|l| l == "MCP_PROBE_CONFIGURED=yes"),
            "{seen:?}"
        );
        assert!(
            seen.iter().any(|l| l == "GITHUB_TOKEN=declared-token"),
            "{seen:?}"
        );
        assert!(
            !seen.iter().any(|l| l.contains("planted-secret")),
            "{seen:?}"
        );
        assert!(
            !seen.iter().any(|l| l.starts_with("CARGO_MANIFEST_DIR=")),
            "the agent's own environment leaked into the MCP server"
        );
    }
}
