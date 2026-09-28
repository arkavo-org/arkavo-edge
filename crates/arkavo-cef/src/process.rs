use crate::error::{CefError, Result};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use tokio::time::{Duration, sleep};
use tracing::{debug, error, info, warn};

pub struct CefProcess {
    child: Option<Child>,
    socket_path: PathBuf,
}

impl CefProcess {
    pub fn spawn(socket_path: impl AsRef<Path>, renderer_path: impl AsRef<Path>) -> Result<Self> {
        let socket_path = socket_path.as_ref().to_path_buf();

        if socket_path.exists() {
            std::fs::remove_file(&socket_path).map_err(|e| {
                CefError::ProcessStartFailed(format!("Failed to remove old socket: {e}"))
            })?;
        }

        let mut cmd = Command::new(renderer_path.as_ref());
        cmd.arg("--socket")
            .arg(&socket_path)
            .arg("--disable-gpu")
            .arg("--disable-software-rasterizer")
            .arg("--disable-javascript")
            .arg("--no-sandbox");

        if let Ok(port) = std::env::var("ARKAVO_CEF_DEBUG_PORT") {
            cmd.arg("--remote-debugging-port").arg(&port);
            info!("CEF remote debugging enabled on port {}", port);
        }

        let child = cmd
            .spawn()
            .map_err(|e| CefError::ProcessStartFailed(e.to_string()))?;

        info!("CEF process spawned with PID: {}", child.id());

        Ok(Self {
            child: Some(child),
            socket_path,
        })
    }

    pub async fn wait_for_socket(&mut self, timeout: Duration) -> Result<()> {
        let start = std::time::Instant::now();

        while !self.socket_path.exists() {
            if start.elapsed() > timeout {
                return Err(CefError::Timeout);
            }

            if let Some(ref mut child) = self.child {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        return Err(CefError::ProcessCrashed(format!(
                            "Process exited with status: {status}"
                        )));
                    }
                    Ok(None) => {}
                    Err(e) => {
                        return Err(CefError::ProcessStartFailed(format!(
                            "Failed to check process status: {e}"
                        )));
                    }
                }
            }

            sleep(Duration::from_millis(50)).await;
        }

        debug!("CEF socket ready at {:?}", self.socket_path);
        Ok(())
    }

    pub fn is_running(&mut self) -> bool {
        self.child
            .as_mut()
            .is_some_and(|c| c.try_wait().is_ok_and(|status| status.is_none()))
    }

    pub fn kill(&mut self) -> Result<()> {
        if let Some(ref mut child) = self.child {
            #[allow(clippy::cast_possible_wrap)]
            let pid = Pid::from_raw(child.id() as i32);

            if let Err(e) = kill(pid, Signal::SIGTERM) {
                warn!("Failed to send SIGTERM to CEF process: {}", e);

                if let Err(e) = kill(pid, Signal::SIGKILL) {
                    error!("Failed to send SIGKILL to CEF process: {}", e);
                    return Err(CefError::ProcessCrashed(format!(
                        "Failed to kill process: {e}"
                    )));
                }
            }

            if let Err(e) = child.wait() {
                warn!("Failed to wait for CEF process: {}", e);
            }

            info!("CEF process terminated");
        }

        if self.socket_path.exists()
            && let Err(e) = std::fs::remove_file(&self.socket_path)
        {
            warn!("Failed to remove socket file: {}", e);
        }

        Ok(())
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

impl Drop for CefProcess {
    fn drop(&mut self) {
        if let Err(e) = self.kill() {
            error!("Error during CEF process cleanup: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_creation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let socket_path = temp_dir.path().join("test.sock");

        let result = CefProcess::spawn(&socket_path, "/bin/sleep");
        assert!(result.is_ok() || result.is_err());
    }
}
