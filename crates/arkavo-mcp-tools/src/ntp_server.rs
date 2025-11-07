use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

const NTP_PACKET_SIZE: usize = 48;
const NTP_EPOCH_OFFSET: u64 = 2_208_988_800;

#[derive(Debug, Clone)]
pub struct NtpServerStats {
    pub requests_received: u64,
    pub requests_served: u64,
    pub errors: u64,
    pub start_time: SystemTime,
}

pub struct NtpServer {
    bind_addr: SocketAddr,
    stratum: u8,
    running: Arc<AtomicBool>,
    stats: Arc<RwLock<NtpServerStats>>,
    task_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
    request_count: Arc<AtomicU64>,
    served_count: Arc<AtomicU64>,
    error_count: Arc<AtomicU64>,
}

impl NtpServer {
    pub fn new(bind_addr: SocketAddr, stratum: u8) -> Self {
        Self {
            bind_addr,
            stratum: stratum.clamp(1, 15),
            running: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(RwLock::new(NtpServerStats {
                requests_received: 0,
                requests_served: 0,
                errors: 0,
                start_time: SystemTime::now(),
            })),
            task_handle: Arc::new(RwLock::new(None)),
            request_count: Arc::new(AtomicU64::new(0)),
            served_count: Arc::new(AtomicU64::new(0)),
            error_count: Arc::new(AtomicU64::new(0)),
        }
    }

    pub async fn start(&self) -> Result<(), std::io::Error> {
        if self.running.load(Ordering::SeqCst) {
            warn!("NTP server already running");
            return Ok(());
        }

        let socket = UdpSocket::bind(self.bind_addr).await?;
        info!("NTP server listening on {}", self.bind_addr);

        self.running.store(true, Ordering::SeqCst);

        let running = Arc::clone(&self.running);
        let stratum = self.stratum;
        let request_count = Arc::clone(&self.request_count);
        let served_count = Arc::clone(&self.served_count);
        let error_count = Arc::clone(&self.error_count);

        let handle = tokio::spawn(async move {
            let mut buf = [0u8; NTP_PACKET_SIZE];

            while running.load(Ordering::SeqCst) {
                match socket.recv_from(&mut buf).await {
                    Ok((len, peer_addr)) => {
                        request_count.fetch_add(1, Ordering::Relaxed);

                        if len < NTP_PACKET_SIZE {
                            warn!(
                                "Received malformed NTP packet from {}: size {}",
                                peer_addr, len
                            );
                            error_count.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }

                        debug!("Received NTP request from {}", peer_addr);

                        match Self::handle_ntp_request(&buf, stratum) {
                            Ok(response) => match socket.send_to(&response, peer_addr).await {
                                Ok(_) => {
                                    served_count.fetch_add(1, Ordering::Relaxed);
                                    debug!("Sent NTP response to {}", peer_addr);
                                }
                                Err(e) => {
                                    error!("Failed to send NTP response to {}: {}", peer_addr, e);
                                    error_count.fetch_add(1, Ordering::Relaxed);
                                }
                            },
                            Err(e) => {
                                warn!("Failed to generate NTP response for {}: {}", peer_addr, e);
                                error_count.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Error receiving NTP packet: {}", e);
                        error_count.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }

            info!("NTP server stopped");
        });

        *self.task_handle.write().await = Some(handle);

        Ok(())
    }

    pub async fn stop(&self) {
        if !self.running.load(Ordering::SeqCst) {
            return;
        }

        info!("Stopping NTP server...");
        self.running.store(false, Ordering::SeqCst);

        let handle = self.task_handle.write().await.take();
        if let Some(h) = handle {
            let _ = h.await;
        }
    }

    pub async fn get_stats(&self) -> NtpServerStats {
        let mut stats = self.stats.write().await;
        stats.requests_received = self.request_count.load(Ordering::Relaxed);
        stats.requests_served = self.served_count.load(Ordering::Relaxed);
        stats.errors = self.error_count.load(Ordering::Relaxed);
        stats.clone()
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn handle_ntp_request(request: &[u8], stratum: u8) -> Result<[u8; NTP_PACKET_SIZE], String> {
        let mode = request[0] & 0x07;

        if mode != 3 {
            return Err(format!("Invalid NTP mode: {}", mode));
        }

        let mut response = [0u8; NTP_PACKET_SIZE];

        response[0] = 0x24;
        response[1] = stratum;
        response[2] = request[2];
        response[3] = 0xEC;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("System time error: {}", e))?;

        let ntp_seconds = now.as_secs() + NTP_EPOCH_OFFSET;
        let ntp_fraction = ((now.subsec_nanos() as u64) << 32) / 1_000_000_000;

        response[24..32].copy_from_slice(&request[40..48]);

        Self::write_timestamp(&mut response[32..40], ntp_seconds, ntp_fraction);

        Self::write_timestamp(&mut response[40..48], ntp_seconds, ntp_fraction);

        Ok(response)
    }

    fn write_timestamp(dest: &mut [u8], seconds: u64, fraction: u64) {
        dest[0..4].copy_from_slice(&(seconds as u32).to_be_bytes());
        dest[4..8].copy_from_slice(&(fraction as u32).to_be_bytes());
    }
}

impl Drop for NtpServer {
    fn drop(&mut self) {
        if self.running.load(Ordering::SeqCst) {
            self.running.store(false, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ntp_server_creation() {
        let addr = "127.0.0.1:0".parse().unwrap();
        let server = NtpServer::new(addr, 3);
        assert!(!server.is_running());
    }

    #[tokio::test]
    async fn test_ntp_server_start_stop() {
        let addr = "127.0.0.1:0".parse().unwrap();
        let server = NtpServer::new(addr, 3);

        assert!(server.start().await.is_ok());
        assert!(server.is_running());

        server.stop().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        assert!(!server.is_running());
    }

    #[tokio::test]
    async fn test_ntp_server_stats() {
        let addr = "127.0.0.1:0".parse().unwrap();
        let server = NtpServer::new(addr, 3);

        let stats = server.get_stats().await;
        assert_eq!(stats.requests_received, 0);
        assert_eq!(stats.requests_served, 0);
        assert_eq!(stats.errors, 0);
    }

    #[test]
    fn test_handle_ntp_request() {
        let mut request = [0u8; NTP_PACKET_SIZE];
        request[0] = 0x23;

        let result = NtpServer::handle_ntp_request(&request, 3);
        assert!(result.is_ok());

        let response = result.unwrap();
        assert_eq!(response[0] & 0x38, 0x20);
        assert_eq!(response[1], 3);
    }

    #[test]
    fn test_invalid_ntp_mode() {
        let mut request = [0u8; NTP_PACKET_SIZE];
        request[0] = 0x20;

        let result = NtpServer::handle_ntp_request(&request, 3);
        assert!(result.is_err());
    }

    #[test]
    fn test_stratum_clamping() {
        let addr = "127.0.0.1:0".parse().unwrap();
        let server = NtpServer::new(addr, 20);
        assert_eq!(server.stratum, 15);

        let server = NtpServer::new(addr, 0);
        assert_eq!(server.stratum, 1);
    }

    #[test]
    fn test_write_timestamp() {
        let mut dest = [0u8; 8];
        NtpServer::write_timestamp(&mut dest, 3_900_000_000, 0x80000000);

        let seconds = u32::from_be_bytes([dest[0], dest[1], dest[2], dest[3]]);
        let fraction = u32::from_be_bytes([dest[4], dest[5], dest[6], dest[7]]);

        assert_eq!(seconds, 3_900_000_000);
        assert_eq!(fraction, 0x80000000);
    }
}
