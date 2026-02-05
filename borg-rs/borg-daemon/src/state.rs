//! Shared job state tracking for the daemon

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Runtime state for a backup job
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobState {
    pub last_run: Option<DateTime<Utc>>,
    pub last_success: Option<DateTime<Utc>>,
    pub last_failure: Option<DateTime<Utc>>,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
    pub last_failure_alert: Option<DateTime<Utc>>,
    pub last_missed_alert: Option<DateTime<Utc>>,
}

impl JobState {
    pub fn record_success(&mut self, now: DateTime<Utc>) {
        self.last_run = Some(now);
        self.last_success = Some(now);
        self.last_failure = None;
        self.consecutive_failures = 0;
        self.last_error = None;
    }

    pub fn record_failure(&mut self, now: DateTime<Utc>, error: String) {
        self.last_run = Some(now);
        self.last_failure = Some(now);
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.last_error = Some(error);
    }

    pub fn record_failure_alert(&mut self, now: DateTime<Utc>) {
        self.last_failure_alert = Some(now);
    }

    pub fn record_missed_alert(&mut self, now: DateTime<Utc>) {
        self.last_missed_alert = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_success() {
        let mut state = JobState::default();
        let now = Utc::now();
        state.record_success(now);

        assert_eq!(state.last_run, Some(now));
        assert_eq!(state.last_success, Some(now));
        assert_eq!(state.consecutive_failures, 0);
        assert!(state.last_error.is_none());
    }

    #[test]
    fn test_record_failure() {
        let mut state = JobState::default();
        let now = Utc::now();
        state.record_failure(now, "boom".to_string());

        assert_eq!(state.last_run, Some(now));
        assert_eq!(state.last_failure, Some(now));
        assert_eq!(state.consecutive_failures, 1);
        assert_eq!(state.last_error.as_deref(), Some("boom"));
    }
}