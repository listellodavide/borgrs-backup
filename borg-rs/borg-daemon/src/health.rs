//! Health monitoring and metrics

use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, info, warn};

use crate::notifications::{self, NotificationEvent, NotificationPayload};
use crate::DaemonState;
use borg_core::db::Database;

/// Health monitor for the daemon
pub struct HealthMonitor {
    state: Arc<DaemonState>,
}

impl HealthMonitor {
    pub fn new(state: Arc<DaemonState>) -> Self {
        Self { state }
    }

    /// Run the health monitor loop
    pub async fn run(&self) {
        let mut shutdown_rx = self.state.shutdown_tx.subscribe();
        let interval = {
            let config = self.state.config.read().await;
            Duration::from_secs(config.global.health_check_interval)
        };

        info!("Health monitor started with {:?} interval", interval);

        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {
                    self.perform_health_check().await;
                }
                _ = shutdown_rx.recv() => {
                    info!("Health monitor shutting down");
                    break;
                }
            }
        }
    }

    /// Perform a health check
    async fn perform_health_check(&self) {
        debug!("Performing health check");

        self.write_heartbeat().await;
        self.check_missed_jobs().await;

        // Check running jobs
        let running_jobs = self.state.running_jobs.read().await;
        if !running_jobs.is_empty() {
            debug!("Running jobs: {:?}", *running_jobs);
        }

        // Notify systemd watchdog if configured
        #[cfg(unix)]
        {
            let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog]);
        }

        // Check repository connectivity for scheduled jobs
        // (In a real implementation, we'd check repositories periodically)

        debug!("Health check completed");
    }

    async fn write_heartbeat(&self) {
        let heartbeat_path = {
            let config = self.state.config.read().await;
            config.global.heartbeat_path.clone()
        };

        if let Some(path) = heartbeat_path {
            if let Some(parent) = path.parent() {
                if let Err(err) = tokio::fs::create_dir_all(parent).await {
                    warn!("Failed to create heartbeat directory: {}", err);
                    return;
                }
            }

            let timestamp = chrono::Utc::now().to_rfc3339();
            if let Err(err) = tokio::fs::write(&path, timestamp).await {
                warn!("Failed to write heartbeat file: {}", err);
            }
        }
    }

    async fn check_missed_jobs(&self) {
        let now = chrono::Utc::now();
        let (configs, scheduler) = {
            let config = self.state.config.read().await;
            let scheduler = self.state.scheduler.read().await;
            (config.jobs.clone(), scheduler)
        };

        for job in configs {
            if let Some(threshold_minutes) = job.notifications.missed_threshold_minutes {
                let last_run = {
                    let state = self.state.job_state.read().await;
                    state.get(&job.name).and_then(|s| s.last_run)
                };

                let reference = last_run
                    .unwrap_or_else(|| now - chrono::Duration::minutes(threshold_minutes as i64));
                let expected = scheduler.expected_runs(&job.name, reference, now);

                if expected.is_empty() {
                    continue;
                }

                let overdue = now - expected.last().unwrap();
                if overdue.num_minutes().abs() as u64 >= threshold_minutes {
                    self.send_missed_alert(&job.name, expected.last().unwrap(), last_run)
                        .await;
                }
            }
        }
    }

    async fn send_missed_alert(
        &self,
        job_name: &str,
        expected: &chrono::DateTime<chrono::Utc>,
        last_run: Option<chrono::DateTime<chrono::Utc>>,
    ) {
        let mut job_state = self.state.job_state.write().await;
        let state = job_state.entry(job_name.to_string()).or_default();
        let now = chrono::Utc::now();

        if state
            .last_missed_alert
            .is_some_and(|last| now - last < chrono::Duration::minutes(5))
        {
            return;
        }

        state.record_missed_alert(now);

        let config = self.state.config.read().await;
        if let Some(job) = config.jobs.iter().find(|j| j.name == job_name) {
            if let Some(dispatcher) = notifications::build_dispatcher(&job.notifications) {
                if let Ok(payload) = NotificationPayload::new(NotificationEvent::JobMissed {
                    job: job_name.to_string(),
                    expected: expected.to_rfc3339(),
                    last_run: last_run.map(|t| t.to_rfc3339()),
                }) {
                    dispatcher.send(payload, &job.retry).await;
                }
            }
        }
    }
}

/// Metrics collector for Prometheus
pub struct MetricsCollector {
    state: Arc<DaemonState>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;

    #[tokio::test]
    async fn test_write_heartbeat() {
        let temp_dir = tempfile::tempdir().unwrap();
        let heartbeat_path = temp_dir.path().join("heartbeat");

        let config = crate::config::DaemonConfig {
            global: crate::config::GlobalConfig {
                heartbeat_path: Some(heartbeat_path.clone()),
                ..Default::default()
            },
            jobs: vec![],
            repositories: vec![],
        };

        let (shutdown_tx, _) = broadcast::channel(1);
        let tmp_file = tempfile::NamedTempFile::new().unwrap();
        let db = Arc::new(Database::new(tmp_file.path()).await.unwrap());
        let state = Arc::new(crate::DaemonState::new(config, shutdown_tx, db));
        let monitor = HealthMonitor::new(state);

        monitor.write_heartbeat().await;
        let contents = tokio::fs::read_to_string(&heartbeat_path).await.unwrap();
        assert!(contents.contains('T'));
    }
}

impl MetricsCollector {
    pub fn new(state: Arc<DaemonState>) -> Self {
        Self { state }
    }

    /// Generate Prometheus metrics
    pub async fn collect(&self) -> String {
        let running = self.state.running_jobs.read().await;
        let scheduler = self.state.scheduler.read().await;
        let scheduled = scheduler.list_scheduled();
        let job_state = self.state.job_state.read().await;

        let mut output = String::new();

        // Running jobs gauge
        output
            .push_str("# HELP borgd_running_jobs_total Number of currently running backup jobs\n");
        output.push_str("# TYPE borgd_running_jobs_total gauge\n");
        output.push_str(&format!("borgd_running_jobs_total {}\n\n", running.len()));

        // Scheduled jobs gauge
        output.push_str("# HELP borgd_scheduled_jobs_total Number of scheduled backup jobs\n");
        output.push_str("# TYPE borgd_scheduled_jobs_total gauge\n");
        output.push_str(&format!(
            "borgd_scheduled_jobs_total {}\n\n",
            scheduled.len()
        ));

        // Per-job metrics
        output.push_str("# HELP borgd_job_enabled Whether a backup job is enabled\n");
        output.push_str("# TYPE borgd_job_enabled gauge\n");
        for (name, _, enabled) in &scheduled {
            let value = if *enabled { 1 } else { 0 };
            output.push_str(&format!(
                "borgd_job_enabled{{job=\"{}\"}} {}\n",
                name, value
            ));
        }

        // Last success/failure timestamps (seconds since epoch)
        output.push_str("\n# HELP borgd_job_last_success Last successful run timestamp\n");
        output.push_str("# TYPE borgd_job_last_success gauge\n");
        for (name, _, _) in &scheduled {
            let ts = job_state
                .get(name)
                .and_then(|state| state.last_success)
                .map(|t| t.timestamp())
                .unwrap_or(0);
            output.push_str(&format!(
                "borgd_job_last_success{{job=\"{}\"}} {}\n",
                name, ts
            ));
        }

        output.push_str("\n# HELP borgd_job_last_failure Last failed run timestamp\n");
        output.push_str("# TYPE borgd_job_last_failure gauge\n");
        for (name, _, _) in &scheduled {
            let ts = job_state
                .get(name)
                .and_then(|state| state.last_failure)
                .map(|t| t.timestamp())
                .unwrap_or(0);
            output.push_str(&format!(
                "borgd_job_last_failure{{job=\"{}\"}} {}\n",
                name, ts
            ));
        }

        output
    }
}
