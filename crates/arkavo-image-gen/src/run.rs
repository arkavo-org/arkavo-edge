//! Running sd-cli: isolated, sandboxed on macOS, killed at its deadline
//! (IMG-006, IMG-012).

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant as StdInstant;

use arkavo_process_env::{ChildEnv, EnvSpec};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::time::Instant;

use crate::error::{ImageGenError, Result};
use crate::progress::{Event, Parser};

const TAIL_BYTES: usize = 4096;

/// Deny the network; allow writes only to the output directory and the
/// per-user temporary and cache directories Metal's shader cache uses.
const MACOS_PROFILE: &str = r#"(version 1)
(allow default)
(deny network*)
(deny file-write*)
(allow file-write*
    (subpath (param "OUT"))
    (subpath (param "USERDIR")))
(allow file-write-data
    (literal "/dev/null")
    (literal "/dev/dtracehelper")
    (literal "/dev/tty"))
"#;

/// Whether the runtime starts under the macOS sandbox profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sandbox {
    MacOs,
    /// Only for runtimes started outside a qualifying host, such as tests.
    Off,
}

/// One sd-cli invocation.
#[derive(Debug)]
pub struct RunSpec<'a> {
    pub cli: &'a Path,
    pub args: Vec<OsString>,
    pub out_dir: &'a Path,
    pub steps: u32,
    pub deadline: Instant,
    pub sandbox: Sandbox,
}

/// What a finished run reported.
#[derive(Debug, Default)]
pub struct RunOutput {
    /// The seed of each image, in order.
    pub seeds: Vec<u32>,
    /// Mean seconds per denoising step, when steps were seen.
    pub seconds_per_step: Option<f64>,
    /// The end of what the process printed.
    pub tail: String,
}

#[derive(Default)]
struct State {
    parser: Option<Parser>,
    tail: VecDeque<u8>,
    seeds: Vec<u32>,
    last_step: Option<StdInstant>,
    step_total: f64,
    step_count: u32,
}

/// Run to completion, reporting events as they are printed.
pub async fn run(spec: RunSpec<'_>, on_event: &(dyn Fn(Event) + Send + Sync)) -> Result<RunOutput> {
    // Boxed: the child, both readers and the timer make a future of
    // several kilobytes that would otherwise sit in every caller's.
    Box::pin(run_inner(spec, on_event)).await
}

async fn run_inner(
    spec: RunSpec<'_>,
    on_event: &(dyn Fn(Event) + Send + Sync),
) -> Result<RunOutput> {
    // The arguments name files and sizes only; the prompt is in a file.
    tracing::debug!(cli = %spec.cli.display(), args = ?spec.args, "starting sd-cli");
    let env = ChildEnv::isolated_from_current(&EnvSpec::default());
    let mut command = match spec.sandbox {
        Sandbox::MacOs => {
            let mut command = env.command("/usr/bin/sandbox-exec");
            command
                .arg("-D")
                .arg(param("OUT", &canonical(spec.out_dir)))
                .arg("-D")
                .arg(param("USERDIR", &user_dir()))
                .arg("-p")
                .arg(MACOS_PROFILE)
                .arg(spec.cli);
            command
        }
        Sandbox::Off => env.command(spec.cli),
    };
    command.args(&spec.args).current_dir(spec.out_dir);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut command = tokio::process::Command::from(command);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let state = Mutex::new(State {
        parser: Some(Parser::new(spec.steps)),
        ..State::default()
    });
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let pid = child.id();
    let waited = Box::pin(tokio::time::timeout_at(spec.deadline, async {
        let (status, (), ()) = tokio::join!(
            child.wait(),
            drain(stdout, &state, on_event),
            drain(stderr, &state, on_event)
        );
        status
    }))
    .await;
    let status = match waited {
        Ok(status) => status?,
        Err(_) => {
            kill_group(pid);
            let _ = child.kill().await;
            return Err(ImageGenError::TimedOut);
        }
    };
    let state = state
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tail =
        String::from_utf8_lossy(&state.tail.iter().copied().collect::<Vec<_>>()).into_owned();
    if !status.success() {
        return Err(ImageGenError::Runtime {
            status: status.to_string(),
            stderr: tail,
        });
    }
    Ok(RunOutput {
        seeds: state.seeds,
        seconds_per_step: (state.step_count > 0)
            .then(|| state.step_total / f64::from(state.step_count)),
        tail,
    })
}

/// Read a stream to its end, keeping the tail and reporting each event as
/// its line completes.
async fn drain(
    stream: Option<impl AsyncRead + Unpin>,
    state: &Mutex<State>,
    on_event: &(dyn Fn(Event) + Send + Sync),
) {
    let Some(mut stream) = stream else {
        return;
    };
    let mut line = Vec::new();
    let mut buffer = [0u8; 8192];
    while let Ok(read) = stream.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        let mut events = Vec::new();
        let mut guard = lock(state);
        for &byte in &buffer[..read] {
            guard.tail.push_back(byte);
            if guard.tail.len() > TAIL_BYTES {
                guard.tail.pop_front();
            }
            if byte == b'\n' || byte == b'\r' {
                events.extend(parse(&mut guard, &line));
                line.clear();
            } else {
                line.push(byte);
            }
        }
        drop(guard);
        for event in events {
            on_event(event);
        }
    }
    let last = parse(&mut lock(state), &line);
    if let Some(event) = last {
        on_event(event);
    }
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn parse(state: &mut State, line: &[u8]) -> Option<Event> {
    let text = String::from_utf8_lossy(line);
    let event = state.parser.as_mut()?.line(&text)?;
    match event {
        Event::ImageStarted { seed, .. } => {
            state.seeds.push(seed);
            state.last_step = Some(StdInstant::now());
        }
        Event::Step { .. } => {
            let now = StdInstant::now();
            if let Some(last) = state.last_step.replace(now) {
                state.step_total += now.duration_since(last).as_secs_f64();
                state.step_count += 1;
            }
        }
    }
    Some(event)
}

fn param(name: &str, value: &Path) -> OsString {
    let mut param = OsString::from(format!("{name}="));
    param.push(value);
    param
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The per-user directory holding TMPDIR and the cache directory beside it
/// (/private/var/folders/xx/yyyy on macOS).
fn user_dir() -> PathBuf {
    let tmp = canonical(&std::env::temp_dir());
    tmp.parent().map_or_else(|| tmp.clone(), Path::to_path_buf)
}

#[cfg(unix)]
fn kill_group(pid: Option<u32>) {
    if let Some(pid) = pid.and_then(|p| i32::try_from(p).ok()) {
        // SAFETY: kill has no memory effects; a negative pid names the
        // process group the child leads (process_group(0) above).
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
const fn kill_group(_pid: Option<u32>) {}

#[cfg(test)]
mod tests;
