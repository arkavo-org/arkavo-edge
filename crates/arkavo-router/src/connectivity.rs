use std::time::Duration;

pub struct ConnectivityChecker {
    timeout: Duration,
}

impl ConnectivityChecker {
    pub fn new() -> Self {
        Self {
            timeout: Duration::from_secs(2),
        }
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        Self { timeout }
    }

    pub async fn is_online(&self) -> bool {
        self.check_connectivity().await
    }

    async fn check_connectivity(&self) -> bool {
        let hosts = vec![
            "https://generativelanguage.googleapis.com",
            "https://www.google.com",
            "https://1.1.1.1",
        ];

        for host in hosts {
            if let Ok(result) = tokio::time::timeout(self.timeout, reqwest::get(host)).await
                && result.is_ok() {
                    tracing::debug!("Online: Connectivity check succeeded to {}", host);
                    return true;
                }
        }

        tracing::info!("Offline: All connectivity checks failed");
        false
    }

    pub async fn check_api_availability(&self, api_url: &str) -> bool {
        match tokio::time::timeout(self.timeout, reqwest::Client::new().head(api_url).send()).await
        {
            Ok(Ok(response)) => {
                let available =
                    response.status().is_success() || response.status().is_redirection();
                tracing::debug!("API {} availability: {}", api_url, available);
                available
            }
            Ok(Err(e)) => {
                tracing::debug!("API {} unavailable: {}", api_url, e);
                false
            }
            Err(_) => {
                tracing::debug!("API {} check timed out", api_url);
                false
            }
        }
    }
}

impl Default for ConnectivityChecker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connectivity_checker_creation() {
        let checker = ConnectivityChecker::new();
        assert_eq!(checker.timeout, Duration::from_secs(2));
    }

    #[tokio::test]
    async fn test_custom_timeout() {
        let checker = ConnectivityChecker::with_timeout(Duration::from_secs(5));
        assert_eq!(checker.timeout, Duration::from_secs(5));
    }
}
