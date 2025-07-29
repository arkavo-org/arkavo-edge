use reqwest::Client;
use std::fmt;
use std::time::{Duration, Instant};
use tokio::time::sleep;

#[derive(Debug)]
pub enum WaitError {
    Timeout {
        elapsed: Duration,
        last_state: String,
    },
    WrongState {
        expected: String,
        actual: String,
    },
    NetworkError(reqwest::Error),
}

impl fmt::Display for WaitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WaitError::Timeout {
                elapsed,
                last_state,
            } => {
                write!(f, "Timeout after {:?}. Last state: {}", elapsed, last_state)
            }
            WaitError::WrongState { expected, actual } => {
                write!(f, "Wrong state. Expected: {}, Actual: {}", expected, actual)
            }
            WaitError::NetworkError(e) => write!(f, "Network error: {}", e),
        }
    }
}

impl std::error::Error for WaitError {}

pub async fn wait_for_ready(url: &str, timeout: Duration) -> Result<(), WaitError> {
    let client = Client::new();
    let start = Instant::now();
    let mut last_error = String::from("Not started");
    let mut backoff = Duration::from_millis(100);

    while start.elapsed() < timeout {
        match client.get(url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    return Ok(());
                }
                last_error = format!("HTTP {}", response.status());
            }
            Err(e) => {
                last_error = format!("Connection error: {}", e);
            }
        }

        sleep(backoff).await;

        backoff = (backoff * 2).min(Duration::from_secs(2));
    }

    Err(WaitError::Timeout {
        elapsed: start.elapsed(),
        last_state: last_error,
    })
}

pub async fn wait_for_condition<F, Fut>(
    mut condition: F,
    timeout: Duration,
    description: &str,
) -> Result<(), WaitError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<bool, String>>,
{
    let start = Instant::now();
    let mut last_state = String::from("Not checked");
    let mut backoff = Duration::from_millis(100);

    while start.elapsed() < timeout {
        match condition().await {
            Ok(true) => return Ok(()),
            Ok(false) => last_state = format!("{} not satisfied", description),
            Err(e) => last_state = e,
        }

        sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(2));
    }

    Err(WaitError::Timeout {
        elapsed: start.elapsed(),
        last_state,
    })
}
