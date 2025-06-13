use crate::{Result, TestError};
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::net::TcpListener;
use std::process::Command;
use std::sync::Mutex;

static ALLOCATED_PORTS: Lazy<Mutex<HashSet<u16>>> = Lazy::new(|| Mutex::new(HashSet::new()));

pub struct IdbPortManager;

impl IdbPortManager {
    /// Check if a port is available for binding
    pub fn is_port_available(port: u16) -> bool {
        TcpListener::bind(("127.0.0.1", port)).is_ok()
    }

    /// Find an available port in a range
    pub fn find_available_port(start: u16, end: u16) -> Option<u16> {
        let allocated = ALLOCATED_PORTS.lock().unwrap();

        (start..=end).find(|&port| !allocated.contains(&port) && Self::is_port_available(port))
    }

    /// Allocate a port (mark it as in use)
    pub fn allocate_port(port: u16) {
        let mut allocated = ALLOCATED_PORTS.lock().unwrap();
        allocated.insert(port);
    }

    /// Release a port (mark it as available)
    pub fn release_port(port: u16) {
        let mut allocated = ALLOCATED_PORTS.lock().unwrap();
        allocated.remove(&port);
    }

    /// Kill any existing IDB companion process on a port
    pub fn kill_idb_on_port(port: u16) -> Result<()> {
        eprintln!(
            "[IdbPortManager] Checking for IDB companion on port {}...",
            port
        );

        // First try lsof to find the process
        let lsof_output = Command::new("lsof")
            .args(["-i", &format!("tcp:{}", port)])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run lsof: {}", e)))?;

        if lsof_output.status.success() {
            let output = String::from_utf8_lossy(&lsof_output.stdout);

            // Parse lsof output to find idb_companion processes
            for line in output.lines().skip(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() > 1 && parts[0].contains("idb_compan") {
                    let pid = parts[1];
                    eprintln!(
                        "[IdbPortManager] Found idb_companion process {} on port {}, killing...",
                        pid, port
                    );

                    let _ = Command::new("kill").arg("-9").arg(pid).output();

                    // Wait a bit for the process to die
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
        }

        // Also try to kill all idb_companion processes as a fallback
        let _ = Command::new("pkill").args(["-9", "idb_companion"]).output();

        Ok(())
    }

    /// Get the next available port for IDB companion, handling conflicts
    pub fn get_next_idb_port() -> Result<u16> {
        let default_port = 10882;
        let max_port = 10892;

        // First, try to clean up any existing IDB on default port
        if !Self::is_port_available(default_port) {
            eprintln!(
                "[IdbPortManager] Default port {} is in use, attempting cleanup...",
                default_port
            );
            Self::kill_idb_on_port(default_port)?;

            // Check again after cleanup
            if Self::is_port_available(default_port) {
                Self::allocate_port(default_port);
                return Ok(default_port);
            }
        } else {
            Self::allocate_port(default_port);
            return Ok(default_port);
        }

        // If default port is still not available, find another
        if let Some(port) = Self::find_available_port(default_port + 1, max_port) {
            eprintln!("[IdbPortManager] Using alternate port: {}", port);
            Self::allocate_port(port);
            Ok(port)
        } else {
            Err(TestError::Mcp(
                "No available ports for IDB companion".to_string(),
            ))
        }
    }
}
