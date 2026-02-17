//! Session Timeout Enforcement
//!
//! Implements absolute and idle timeout enforcement for sessions.
//!
//! ## Spec Coverage
//! - [specs/arkavo-edge/session-security.spec.yaml](SESS-001): Absolute session timeout
//! - [specs/arkavo-edge/session-security.spec.yaml](SESS-002): Idle session timeout  
//! - [specs/arkavo-edge/session-security.spec.yaml](SESS-003): Timeout configuration validation

use std::time::{Duration, Instant};

/// Configuration for session timeouts
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeoutConfig {
    /// Maximum session lifetime (absolute timeout)
    pub absolute_timeout: Duration,
    /// Maximum idle time before termination
    pub idle_timeout: Duration,
    /// Check interval for idle timeout cleanup
    pub check_interval: Duration,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            absolute_timeout: Duration::from_secs(3600), // 1 hour
            idle_timeout: Duration::from_secs(900),      // 15 minutes
            check_interval: Duration::from_secs(60),     // 1 minute
        }
    }
}

impl TimeoutConfig {
    /// Creates a new timeout config with validation
    ///
    /// ## Errors
    /// Returns `TimeoutConfigError` if invalid values are provided
    ///
    /// ## Validation Rules (SESS-003)
    /// - absolute_timeout must be >= 60 seconds and <= 86400 seconds (24 hours)
    /// - idle_timeout must be >= 60 seconds
    /// - idle_timeout must be <= absolute_timeout
    /// - idle_timeout should be <= 14400 seconds (4 hours) for security
    pub fn new(
        absolute_timeout_secs: u64,
        idle_timeout_secs: u64,
    ) -> Result<Self, TimeoutConfigError> {
        // SESS-003: Validate absolute timeout >= 1 minute
        if absolute_timeout_secs < 60 {
            return Err(TimeoutConfigError::AbsoluteTimeoutTooShort);
        }

        // SESS-003: Validate absolute timeout <= 24 hours
        if absolute_timeout_secs > 86400 {
            return Err(TimeoutConfigError::AbsoluteTimeoutTooLong);
        }

        // SESS-003: Validate idle timeout >= 1 minute
        if idle_timeout_secs < 60 {
            return Err(TimeoutConfigError::IdleTimeoutTooShort);
        }

        // SESS-003: Validate idle timeout <= absolute timeout
        if idle_timeout_secs > absolute_timeout_secs {
            return Err(TimeoutConfigError::IdleTimeoutExceedsAbsolute);
        }

        // SESS-003: Security policy - idle timeout should not exceed 4 hours
        if idle_timeout_secs > 14400 {
            return Err(TimeoutConfigError::IdleTimeoutTooLong);
        }

        Ok(Self {
            absolute_timeout: Duration::from_secs(absolute_timeout_secs),
            idle_timeout: Duration::from_secs(idle_timeout_secs),
            check_interval: Duration::from_secs(60),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TimeoutConfigError {
    AbsoluteTimeoutTooShort,
    AbsoluteTimeoutTooLong,
    IdleTimeoutTooShort,
    IdleTimeoutExceedsAbsolute,
    IdleTimeoutTooLong,
}

impl std::fmt::Display for TimeoutConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TimeoutConfigError::AbsoluteTimeoutTooShort => {
                write!(f, "absolute timeout must be at least 1 minute")
            }
            TimeoutConfigError::AbsoluteTimeoutTooLong => {
                write!(f, "absolute timeout cannot exceed 24 hours")
            }
            TimeoutConfigError::IdleTimeoutTooShort => {
                write!(f, "idle timeout must be at least 1 minute")
            }
            TimeoutConfigError::IdleTimeoutExceedsAbsolute => {
                write!(f, "idle timeout cannot exceed absolute timeout")
            }
            TimeoutConfigError::IdleTimeoutTooLong => {
                write!(f, "idle timeout should not exceed 4 hours for security")
            }
        }
    }
}

impl std::error::Error for TimeoutConfigError {}

/// Tracks timeout state for a session
#[derive(Debug)]
pub struct TimeoutTracker {
    config: TimeoutConfig,
    created_at: Instant,
    last_activity: Instant,
}

impl TimeoutTracker {
    /// Creates a new timeout tracker with the given configuration
    pub fn new(config: TimeoutConfig) -> Self {
        let now = Instant::now();
        Self {
            config,
            created_at: now,
            last_activity: now,
        }
    }

    /// Records activity to prevent idle timeout
    pub fn record_activity(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Checks if the session has timed out (absolute or idle)
    ///
    /// ## Spec
    /// SESS-001: Absolute session timeout enforced
    /// SESS-002: Idle session timeout enforced
    ///
    /// ## Returns
    /// - `TimeoutStatus::Active` if session is still valid
    /// - `TimeoutStatus::AbsoluteExpired` if absolute timeout exceeded
    /// - `TimeoutStatus::IdleExpired` if idle timeout exceeded
    pub fn check_timeout(&self) -> TimeoutStatus {
        let now = Instant::now();

        // SESS-001: Check absolute timeout (session lifetime)
        let absolute_elapsed = now.duration_since(self.created_at);
        if absolute_elapsed >= self.config.absolute_timeout {
            return TimeoutStatus::AbsoluteExpired;
        }

        // SESS-002: Check idle timeout (inactivity)
        let idle_elapsed = now.duration_since(self.last_activity);
        if idle_elapsed >= self.config.idle_timeout {
            return TimeoutStatus::IdleExpired;
        }

        TimeoutStatus::Active
    }

    /// Returns remaining time before absolute timeout
    pub fn remaining_absolute_time(&self) -> Duration {
        let elapsed = self.created_at.elapsed();
        self.config.absolute_timeout.saturating_sub(elapsed)
    }

    /// Returns remaining time before idle timeout
    pub fn remaining_idle_time(&self) -> Duration {
        let elapsed = self.last_activity.elapsed();
        self.config.idle_timeout.saturating_sub(elapsed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeoutStatus {
    Active,
    AbsoluteExpired,
    IdleExpired,
}

#[cfg(test)]
mod tests {
    //! TDD Tests for Session Timeout Enforcement
    //!
    //! ## RED Phase - These tests will fail until implemented

    use super::*;
    use arkavo_test_macros::spec;

    // ============================================================================
    // SESS-001: Absolute session timeout enforced
    // ============================================================================

    /// Test: Session expires after absolute timeout
    /// Spec: SESS-001 - Absolute session timeout enforced
    #[spec("SESS-001")]
    #[test]
    fn test_absolute_timeout_expires_session() {
        // Arrange: Create tracker with very short absolute timeout
        let config = TimeoutConfig {
            absolute_timeout: Duration::from_millis(50),
            idle_timeout: Duration::from_secs(60),
            check_interval: Duration::from_secs(1),
        };
        let tracker = TimeoutTracker::new(config);

        // Act: Wait for timeout and check
        std::thread::sleep(Duration::from_millis(100));
        let status = tracker.check_timeout();

        // Assert: Should be absolute expired
        assert_eq!(status, TimeoutStatus::AbsoluteExpired);
    }

    /// Test: Session is active before absolute timeout
    /// Spec: SESS-001 - Absolute timeout should not trigger early
    #[spec("SESS-001")]
    #[test]
    fn test_absolute_timeout_not_expired_yet() {
        // Arrange: Create tracker with long absolute timeout
        let config = TimeoutConfig {
            absolute_timeout: Duration::from_secs(3600),
            idle_timeout: Duration::from_secs(900),
            check_interval: Duration::from_secs(60),
        };
        let tracker = TimeoutTracker::new(config);

        // Act: Check immediately
        let status = tracker.check_timeout();

        // Assert: Should still be active
        assert_eq!(status, TimeoutStatus::Active);
    }

    // ============================================================================
    // SESS-002: Idle session timeout enforced
    // ============================================================================

    /// Test: Session expires after idle timeout
    /// Spec: SESS-002 - Idle session timeout enforced
    #[spec("SESS-002")]
    #[test]
    fn test_idle_timeout_expires_session() {
        // Arrange: Create tracker with short idle timeout
        let config = TimeoutConfig {
            absolute_timeout: Duration::from_secs(3600),
            idle_timeout: Duration::from_millis(50),
            check_interval: Duration::from_secs(1),
        };
        let tracker = TimeoutTracker::new(config);

        // Act: Wait without activity and check
        std::thread::sleep(Duration::from_millis(100));
        let status = tracker.check_timeout();

        // Assert: Should be idle expired
        assert_eq!(status, TimeoutStatus::IdleExpired);
    }

    /// Test: Activity prevents idle timeout
    /// Spec: SESS-002 - Activity resets idle timer
    #[spec("SESS-002")]
    #[test]
    fn test_activity_prevents_idle_timeout() {
        // Arrange: Create tracker with short idle timeout
        let config = TimeoutConfig {
            absolute_timeout: Duration::from_secs(3600),
            idle_timeout: Duration::from_millis(100),
            check_interval: Duration::from_secs(1),
        };
        let mut tracker = TimeoutTracker::new(config);

        // Act: Wait half the timeout, record activity, wait half again
        std::thread::sleep(Duration::from_millis(50));
        tracker.record_activity();
        std::thread::sleep(Duration::from_millis(50));
        let status = tracker.check_timeout();

        // Assert: Should still be active (activity reset the timer)
        assert_eq!(status, TimeoutStatus::Active);
    }

    // ============================================================================
    // SESS-003: Timeout configuration validation
    // ============================================================================

    /// Test: Zero absolute timeout is rejected
    /// Spec: SESS-003 - Invalid timeout values rejected
    #[test]
    fn test_zero_absolute_timeout_rejected() {
        let result = TimeoutConfig::new(0, 900);
        assert!(matches!(
            result,
            Err(TimeoutConfigError::AbsoluteTimeoutTooShort)
        ));
    }

    /// Test: Absolute timeout > 24 hours is rejected
    /// Spec: SESS-003 - Maximum absolute timeout enforced
    #[test]
    fn test_absolute_timeout_too_long_rejected() {
        let result = TimeoutConfig::new(90000, 900); // > 24 hours
        assert!(matches!(
            result,
            Err(TimeoutConfigError::AbsoluteTimeoutTooLong)
        ));
    }

    /// Test: Zero idle timeout is rejected
    /// Spec: SESS-003 - Invalid idle timeout rejected
    #[test]
    fn test_zero_idle_timeout_rejected() {
        let result = TimeoutConfig::new(3600, 0);
        assert!(matches!(
            result,
            Err(TimeoutConfigError::IdleTimeoutTooShort)
        ));
    }

    /// Test: Idle timeout > absolute timeout is rejected
    /// Spec: SESS-003 - Idle cannot exceed absolute
    #[test]
    fn test_idle_timeout_exceeds_absolute_rejected() {
        let result = TimeoutConfig::new(600, 3600); // idle > absolute
        assert!(matches!(
            result,
            Err(TimeoutConfigError::IdleTimeoutExceedsAbsolute)
        ));
    }

    /// Test: Idle timeout > 4 hours is rejected (security policy)
    /// Spec: SESS-003 - Maximum idle timeout for security
    #[test]
    fn test_idle_timeout_too_long_rejected() {
        let result = TimeoutConfig::new(86400, 18000); // idle > 4 hours
        assert!(matches!(
            result,
            Err(TimeoutConfigError::IdleTimeoutTooLong)
        ));
    }

    /// Test: Valid configuration is accepted
    /// Spec: SESS-003 - Valid configurations pass
    #[test]
    fn test_valid_timeout_config_accepted() {
        let result = TimeoutConfig::new(3600, 900);
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.absolute_timeout, Duration::from_secs(3600));
        assert_eq!(config.idle_timeout, Duration::from_secs(900));
    }

    /// Test: Default configuration is secure
    /// Spec: SESS-003 - Defaults should be reasonable
    #[test]
    fn test_default_config_is_secure() {
        let config = TimeoutConfig::default();
        assert_eq!(config.absolute_timeout, Duration::from_secs(3600));
        assert_eq!(config.idle_timeout, Duration::from_secs(900));
        assert_eq!(config.check_interval, Duration::from_secs(60));
    }

    // ============================================================================
    // Utility tests
    // ============================================================================

    /// Test: Remaining time calculations
    #[test]
    fn test_remaining_time_calculations() {
        let config = TimeoutConfig {
            absolute_timeout: Duration::from_secs(3600),
            idle_timeout: Duration::from_secs(900),
            check_interval: Duration::from_secs(60),
        };
        let tracker = TimeoutTracker::new(config);

        // Remaining times should be close to configured values
        assert!(tracker.remaining_absolute_time() <= Duration::from_secs(3600));
        assert!(tracker.remaining_idle_time() <= Duration::from_secs(900));
    }
}
