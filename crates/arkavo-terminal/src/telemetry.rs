use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UIEvent {
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
    pub details: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UITelemetryReport {
    pub session_start: DateTime<Utc>,
    pub session_duration_ms: u64,
    pub layout_mode_switches: u64,
    pub pane_focus_changes: u64,
    pub messages_sent: u64,
    pub messages_received: u64,
    pub key_events: u64,
    pub portrait_mode_time_ms: u64,
    pub tabbed_mode_time_ms: u64,
}

#[derive(Clone)]
pub struct UITelemetry {
    session_start: DateTime<Utc>,
    pub layout_mode_switches: Arc<AtomicU64>,
    pub pane_focus_changes: Arc<AtomicU64>,
    pub messages_sent: Arc<AtomicU64>,
    pub messages_received: Arc<AtomicU64>,
    pub key_events: Arc<AtomicU64>,
    portrait_mode_time_ms: Arc<AtomicU64>,
    tabbed_mode_time_ms: Arc<AtomicU64>,
    last_mode_switch: Arc<std::sync::Mutex<DateTime<Utc>>>,
    is_portrait: Arc<std::sync::Mutex<bool>>,
}

impl UITelemetry {
    pub fn new() -> Self {
        let now = Utc::now();
        Self {
            session_start: now,
            layout_mode_switches: Arc::new(AtomicU64::new(0)),
            pane_focus_changes: Arc::new(AtomicU64::new(0)),
            messages_sent: Arc::new(AtomicU64::new(0)),
            messages_received: Arc::new(AtomicU64::new(0)),
            key_events: Arc::new(AtomicU64::new(0)),
            portrait_mode_time_ms: Arc::new(AtomicU64::new(0)),
            tabbed_mode_time_ms: Arc::new(AtomicU64::new(0)),
            last_mode_switch: Arc::new(std::sync::Mutex::new(now)),
            is_portrait: Arc::new(std::sync::Mutex::new(false)),
        }
    }

    pub fn track_layout_switch(&self, is_portrait: bool) {
        self.layout_mode_switches.fetch_add(1, Ordering::Relaxed);

        let now = Utc::now();
        if let Ok(mut last_switch) = self.last_mode_switch.lock() {
            let duration = (now - *last_switch).num_milliseconds() as u64;

            if let Ok(was_portrait) = self.is_portrait.lock() {
                if *was_portrait {
                    self.portrait_mode_time_ms
                        .fetch_add(duration, Ordering::Relaxed);
                } else {
                    self.tabbed_mode_time_ms
                        .fetch_add(duration, Ordering::Relaxed);
                }
            }

            *last_switch = now;
        }

        if let Ok(mut mode) = self.is_portrait.lock() {
            *mode = is_portrait;
        }
    }

    pub fn track_pane_focus_change(&self) {
        self.pane_focus_changes.fetch_add(1, Ordering::Relaxed);
    }

    pub fn track_message_sent(&self) {
        self.messages_sent.fetch_add(1, Ordering::Relaxed);
    }

    pub fn track_message_received(&self) {
        self.messages_received.fetch_add(1, Ordering::Relaxed);
    }

    pub fn track_key_event(&self) {
        self.key_events.fetch_add(1, Ordering::Relaxed);
    }

    pub fn generate_report(&self) -> UITelemetryReport {
        let now = Utc::now();
        let session_duration = (now - self.session_start).num_milliseconds() as u64;

        // Add remaining time in current mode
        let mut portrait_time = self.portrait_mode_time_ms.load(Ordering::Relaxed);
        let mut tabbed_time = self.tabbed_mode_time_ms.load(Ordering::Relaxed);

        if let Ok(last_switch) = self.last_mode_switch.lock() {
            let remaining_duration = (now - *last_switch).num_milliseconds() as u64;
            if let Ok(is_portrait) = self.is_portrait.lock() {
                if *is_portrait {
                    portrait_time += remaining_duration;
                } else {
                    tabbed_time += remaining_duration;
                }
            }
        }

        UITelemetryReport {
            session_start: self.session_start,
            session_duration_ms: session_duration,
            layout_mode_switches: self.layout_mode_switches.load(Ordering::Relaxed),
            pane_focus_changes: self.pane_focus_changes.load(Ordering::Relaxed),
            messages_sent: self.messages_sent.load(Ordering::Relaxed),
            messages_received: self.messages_received.load(Ordering::Relaxed),
            key_events: self.key_events.load(Ordering::Relaxed),
            portrait_mode_time_ms: portrait_time,
            tabbed_mode_time_ms: tabbed_time,
        }
    }
}

impl Default for UITelemetry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_telemetry_tracking() {
        let telemetry = UITelemetry::new();

        // Track some events
        telemetry.track_key_event();
        telemetry.track_key_event();
        telemetry.track_message_sent();
        telemetry.track_message_received();
        telemetry.track_pane_focus_change();

        // Switch layouts
        telemetry.track_layout_switch(true);
        thread::sleep(Duration::from_millis(100));
        telemetry.track_layout_switch(false);

        let report = telemetry.generate_report();

        assert_eq!(report.key_events, 2);
        assert_eq!(report.messages_sent, 1);
        assert_eq!(report.messages_received, 1);
        assert_eq!(report.pane_focus_changes, 1);
        assert_eq!(report.layout_mode_switches, 2);
        assert!(report.portrait_mode_time_ms >= 100);
    }
}
