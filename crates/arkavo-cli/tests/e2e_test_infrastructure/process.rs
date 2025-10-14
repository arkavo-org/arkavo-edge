use std::fs::File;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use tempfile::TempDir;
use tokio::sync::Mutex;

pub struct TestComponent {
    pub name: String,
    pub process: Arc<Mutex<Child>>,
    pub log_prefix: String,
    pub stdout_file: Arc<Mutex<File>>,
    pub stderr_file: Arc<Mutex<File>>,
    _log_threads: Vec<thread::JoinHandle<()>>,
}

impl TestComponent {
    pub(crate) async fn kill(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut process = self.process.lock().await;
        process.kill()?;
        Ok(())
    }

    pub(crate) async fn is_running(&self) -> bool {
        let mut process = self.process.lock().await;
        match process.try_wait() {
            Ok(None) => true,
            Ok(Some(status)) => {
                eprintln!("{} exited with status: {}", self.name, status);
                false
            }
            Err(e) => {
                eprintln!("{} error checking status: {}", self.name, e);
                false
            }
        }
    }
}

impl Drop for TestComponent {
    fn drop(&mut self) {
        if let Ok(mut process) = self.process.try_lock() {
            let _ = process.kill();
        }
    }
}

pub async fn spawn_component(
    name: &str,
    args: &[&str],
    test_dir: &TempDir,
) -> Result<TestComponent, Box<dyn std::error::Error>> {
    let log_prefix = format!("[{}]", name.to_uppercase());

    let stdout_path = test_dir.path().join(format!("{name}.stdout.log"));
    let stderr_path = test_dir.path().join(format!("{name}.stderr.log"));

    let stdout_file = File::create(&stdout_path)?;
    let stderr_file = File::create(&stderr_path)?;

    // Look for arkavo binary in workspace target directory
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let arkavo_path = workspace_root.join("target").join("debug").join("arkavo");

    if !arkavo_path.exists() {
        return Err(format!("arkavo binary not found at: {arkavo_path:?}").into());
    }

    println!(
        "{} Starting: {} {}",
        log_prefix,
        arkavo_path.display(),
        args.join(" ")
    );

    let mut cmd = Command::new(&arkavo_path);
    cmd.args(args);

    // Add --prompt flag after the command to prevent Terminal.app relaunch on macOS
    if cfg!(target_os = "macos") && !args.contains(&"serve") {
        cmd.arg("--prompt");
    }

    cmd.current_dir(test_dir.path())
        .env("RUST_LOG", "debug")
        .env("TERM", "xterm") // Set terminal type to avoid macOS relaunch
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn {name}: {e}"))?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let log_prefix_clone = log_prefix.clone();
    let stdout_thread = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().flatten() {
            println!("{log_prefix_clone} {line}");
        }
    });

    let log_prefix_clone = log_prefix.clone();
    let stderr_thread = thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().flatten() {
            eprintln!("{log_prefix_clone} {line}");
        }
    });

    Ok(TestComponent {
        name: name.to_string(),
        process: Arc::new(Mutex::new(child)),
        log_prefix,
        stdout_file: Arc::new(Mutex::new(stdout_file)),
        stderr_file: Arc::new(Mutex::new(stderr_file)),
        _log_threads: vec![stdout_thread, stderr_thread],
    })
}
