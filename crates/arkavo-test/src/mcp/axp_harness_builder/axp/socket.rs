use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::Mutex;

/// Manages AXP socket connections and cleanup
#[allow(dead_code)]
pub(crate) struct SocketManager {
    /// Cache of active socket paths (device_id -> socket_path)
    /// Using Mutex instead of RwLock to prevent race conditions
    socket_cache: Arc<Mutex<Option<(String, String)>>>,

    /// Socket directory path
    socket_dir: PathBuf,
}

#[allow(dead_code)]
impl SocketManager {
    pub(crate) fn new() -> Self {
        let socket_dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".arkavo")
            .join("sockets");

        Self {
            socket_cache: Arc::new(Mutex::new(None)),
            socket_dir,
        }
    }

    /// Get socket path for a device and app
    pub(crate) fn get_socket_path(&self, device_id: &str, app_bundle_id: &str) -> PathBuf {
        self.socket_dir.join(format!(
            "arkavo-axp-{}-{}.sock",
            device_id,
            app_bundle_id.replace('.', "_")
        ))
    }

    /// Check if a cached socket exists for the device
    pub(crate) async fn get_cached_socket(&self, device_id: &str) -> Option<String> {
        let cache = self.socket_cache.lock().await;
        cache
            .as_ref()
            .filter(|(cached_id, _)| cached_id == device_id)
            .map(|(_, path)| path.clone())
    }

    /// Update the socket cache
    pub(crate) async fn update_cache(&self, device_id: String, socket_path: String) {
        let mut cache = self.socket_cache.lock().await;
        *cache = Some((device_id, socket_path));
    }

    /// Clear the socket cache
    pub(crate) async fn clear_cache(&self) {
        let mut cache = self.socket_cache.lock().await;
        *cache = None;
    }

    /// Find active AXP socket for a device
    pub(crate) async fn find_active_socket(&self, device_id: &str) -> Option<String> {
        // Check cache first
        if let Some(cached_path) = self.get_cached_socket(device_id).await {
            eprintln!("[SocketManager] Using cached socket for device {device_id}: {cached_path}");
            return Some(cached_path);
        }

        // Ensure socket directory exists
        if let Err(e) = fs::create_dir_all(&self.socket_dir) {
            eprintln!("[SocketManager] Failed to create socket directory: {e}");
            return None;
        }

        // Search for socket files
        if let Ok(entries) = fs::read_dir(&self.socket_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    // Check if socket is for this device
                    if name.starts_with(&format!("arkavo-axp-{device_id}-"))
                        && name.ends_with(".sock")
                    {
                        let socket_path = entry.path();
                        eprintln!(
                            "[SocketManager] Found socket for device {}: {}",
                            device_id,
                            socket_path.display()
                        );

                        // Try to connect to verify it's active
                        #[cfg(unix)]
                        {
                            if let Ok(_stream) = tokio::net::UnixStream::connect(&socket_path).await {
                                eprintln!("[SocketManager] Socket is active");
                                let socket_path_str = socket_path.to_string_lossy().to_string();
                                self.update_cache(device_id.to_string(), socket_path_str.clone())
                                    .await;
                                return Some(socket_path_str);
                            }
                            eprintln!("[SocketManager] Socket exists but is not active");
                        }
                        #[cfg(not(unix))]
                        {
                            eprintln!("[SocketManager] Unix sockets not supported on Windows");
                        }
                    }
                }
            }
        }

        self.clear_cache().await;
        None
    }

    /// Clean up stale socket files
    /// Removes socket files older than the specified duration
    pub(crate) fn cleanup_stale_sockets(&self, max_age: Duration) -> Result<usize, std::io::Error> {
        let mut removed_count = 0;

        if !self.socket_dir.exists() {
            return Ok(0);
        }

        let now = SystemTime::now();

        for entry in fs::read_dir(&self.socket_dir)? {
            let entry = entry?;
            let path = entry.path();

            // Only process .sock files
            if path.extension().and_then(|s| s.to_str()) != Some("sock") {
                continue;
            }

            // Check file age
            if let Ok(metadata) = entry.metadata()
                && let Ok(modified) = metadata.modified()
                && let Ok(age) = now.duration_since(modified)
                && age > max_age
            {
                // Try to remove stale socket
                match fs::remove_file(&path) {
                    Ok(()) => {
                        eprintln!("[SocketManager] Removed stale socket: {}", path.display());
                        removed_count += 1;
                    }
                    Err(e) => {
                        eprintln!(
                            "[SocketManager] Failed to remove stale socket {}: {}",
                            path.display(),
                            e
                        );
                    }
                }
            }
        }

        Ok(removed_count)
    }

    /// Clean up all sockets for a specific device
    pub(crate) fn cleanup_device_sockets(&self, device_id: &str) -> Result<usize, std::io::Error> {
        let mut removed_count = 0;

        if !self.socket_dir.exists() {
            return Ok(0);
        }

        for entry in fs::read_dir(&self.socket_dir)? {
            let entry = entry?;
            let path = entry.path();

            if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && name.starts_with(&format!("arkavo-axp-{device_id}-"))
                && name.ends_with(".sock")
            {
                match fs::remove_file(&path) {
                    Ok(()) => {
                        eprintln!(
                            "[SocketManager] Removed socket for device {}: {}",
                            device_id,
                            path.display()
                        );
                        removed_count += 1;
                    }
                    Err(e) => {
                        eprintln!(
                            "[SocketManager] Failed to remove socket {}: {}",
                            path.display(),
                            e
                        );
                    }
                }
            }
        }

        Ok(removed_count)
    }
}

impl Drop for SocketManager {
    fn drop(&mut self) {
        // Clean up stale sockets older than 1 hour on drop
        if let Err(e) = self.cleanup_stale_sockets(Duration::from_secs(3600)) {
            eprintln!("[SocketManager] Cleanup failed during drop: {e}");
        }
    }
}
