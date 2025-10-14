use crate::error::Result;
use rand::Rng;
use std::future::Future;
use std::time::Duration;
use tracing::warn;

#[derive(Clone, Debug)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Initial delay before first retry
    pub initial_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Exponential backoff multiplier
    pub backoff_factor: f32,
    /// Jitter factor (0.0 to 1.0) to randomize delays
    pub jitter_factor: f32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_millis(1000),
            max_delay: Duration::from_secs(30),
            backoff_factor: 2.0,
            jitter_factor: 0.1,
        }
    }
}

impl RetryConfig {
    /// Set the maximum number of retry attempts
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Set the initial delay before the first retry
    pub fn with_initial_delay(mut self, delay: Duration) -> Self {
        self.initial_delay = delay;
        self
    }

    /// Set the maximum delay between retries
    pub fn with_max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = delay;
        self
    }

    /// Set the exponential backoff factor
    pub fn with_backoff_factor(mut self, factor: f32) -> Self {
        self.backoff_factor = factor;
        self
    }

    /// Set the jitter factor (0.0 to 1.0)
    pub fn with_jitter_factor(mut self, factor: f32) -> Self {
        self.jitter_factor = factor.clamp(0.0, 1.0);
        self
    }
}

pub struct RetryClient {
    config: RetryConfig,
}

impl RetryClient {
    pub fn new(config: RetryConfig) -> Self {
        Self { config }
    }

    pub async fn execute_with_retry<F, Fut, T>(&self, mut operation: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let mut attempt = 0;
        let mut delay = self.config.initial_delay;

        loop {
            match operation().await {
                Ok(result) => return Ok(result),
                Err(error) => {
                    if !error.is_retryable() || attempt >= self.config.max_retries {
                        return Err(error);
                    }

                    attempt += 1;

                    // Use error-specific delay if available
                    let base_delay = error.retry_delay().unwrap_or(delay);

                    // Apply exponential backoff
                    let backoff_delay = if attempt > 1 {
                        Duration::from_secs_f32(
                            base_delay.as_secs_f32()
                                * self
                                    .config
                                    .backoff_factor
                                    .powi(i32::try_from(attempt).unwrap_or(1) - 1),
                        )
                    } else {
                        base_delay
                    };

                    // Add jitter
                    let jittered_delay = self.add_jitter(backoff_delay);

                    // Cap at max delay
                    let final_delay = jittered_delay.min(self.config.max_delay);

                    warn!(
                        "Request failed (attempt {}/{}): {}. Retrying in {:?}",
                        attempt, self.config.max_retries, error, final_delay
                    );

                    tokio::time::sleep(final_delay).await;

                    // Update delay for next iteration
                    delay =
                        Duration::from_secs_f32(delay.as_secs_f32() * self.config.backoff_factor);
                }
            }
        }
    }

    fn add_jitter(&self, delay: Duration) -> Duration {
        let mut rng = rand::thread_rng();
        let jitter_range = delay.as_secs_f32() * self.config.jitter_factor;
        let jitter = rng.gen_range(-jitter_range..=jitter_range);
        let jittered = delay.as_secs_f32() + jitter;
        Duration::from_secs_f32(jittered.max(0.0))
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;
    use crate::error::KimiError;

    #[test]
    fn test_retry_config_builder() {
        let config = RetryConfig::default()
            .with_max_retries(5)
            .with_initial_delay(Duration::from_millis(500))
            .with_max_delay(Duration::from_secs(60))
            .with_backoff_factor(3.0)
            .with_jitter_factor(0.2);

        assert_eq!(config.max_retries, 5);
        assert_eq!(config.initial_delay, Duration::from_millis(500));
        assert_eq!(config.max_delay, Duration::from_secs(60));
        assert!((config.backoff_factor - 3.0).abs() < f32::EPSILON);
        assert!((config.jitter_factor - 0.2).abs() < f32::EPSILON);
    }

    #[test]
    fn test_jitter_factor_clamping() {
        let config1 = RetryConfig::default().with_jitter_factor(1.5);
        assert!((config1.jitter_factor - 1.0).abs() < f32::EPSILON);

        let config2 = RetryConfig::default().with_jitter_factor(-0.5);
        assert!(config2.jitter_factor.abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn test_successful_operation() {
        let client = RetryClient::new(RetryConfig::default());

        let result = client
            .execute_with_retry(|| async { Ok::<_, KimiError>(42) })
            .await;

        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_non_retryable_error() {
        let client = RetryClient::new(RetryConfig::default());
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let attempts_clone = attempts.clone();

        let result = client
            .execute_with_retry(move || {
                attempts_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move {
                    Err::<i32, _>(KimiError::AuthenticationFailed {
                        message: "Invalid API key".to_string(),
                    })
                }
            })
            .await;

        assert!(result.is_err());
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1); // Should not retry
    }

    #[tokio::test]
    async fn test_retryable_error_succeeds() {
        let client = RetryClient::new(RetryConfig {
            initial_delay: Duration::from_millis(10),
            ..Default::default()
        });

        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let attempts_clone = attempts.clone();

        let result = client
            .execute_with_retry(move || {
                let current = attempts_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                async move {
                    if current < 3 {
                        Err(KimiError::Stream("Connection reset".to_string()))
                    } else {
                        Ok(42)
                    }
                }
            })
            .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_max_retries_exceeded() {
        let client = RetryClient::new(RetryConfig {
            max_retries: 2,
            initial_delay: Duration::from_millis(10),
            ..Default::default()
        });

        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let attempts_clone = attempts.clone();

        let result = client
            .execute_with_retry(move || {
                attempts_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move { Err::<i32, _>(KimiError::Stream("Connection reset".to_string())) }
            })
            .await;

        assert!(result.is_err());
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3); // Initial attempt + 2 retries
    }
}
