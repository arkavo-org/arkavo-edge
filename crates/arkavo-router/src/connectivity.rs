use std::time::Duration;

pub struct ConnectivityChecker {
    timeout: Duration,
    /// Fixed answer, bypassing the probe. Set by [`Self::assume`] so callers
    /// that must be deterministic (tests, air-gapped runs) do not have their
    /// behaviour decided by whether a network happens to be reachable.
    assumed: Option<bool>,
}

impl ConnectivityChecker {
    pub fn new() -> Self {
        Self {
            timeout: Duration::from_secs(2),
            assumed: None,
        }
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            assumed: None,
        }
    }

    /// Answer every connectivity question with `online`, without probing.
    pub fn assume(online: bool) -> Self {
        Self {
            timeout: Duration::from_secs(2),
            assumed: Some(online),
        }
    }

    pub async fn is_online(&self) -> bool {
        if let Some(online) = self.assumed {
            return online;
        }
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
                && result.is_ok()
            {
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
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("ROUTER-007")]
    #[tokio::test]
    async fn test_connectivity_checker_creation() {
        let checker = ConnectivityChecker::new();
        assert_eq!(checker.timeout, Duration::from_secs(2));
    }

    #[spec("ROUTER-007")]
    #[tokio::test]
    async fn test_custom_timeout() {
        let checker = ConnectivityChecker::with_timeout(Duration::from_secs(5));
        assert_eq!(checker.timeout, Duration::from_secs(5));
    }
}
