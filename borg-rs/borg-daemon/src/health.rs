//! Daemon health monitoring

use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use crate::DaemonState;

/// Health monitor task
pub struct HealthMonitor {
    state: Arc<DaemonState>,
}

impl HealthMonitor {
    pub fn new(state: Arc<DaemonState>) -> Self {
        Self { state }
    }

    /// Run the health monitor loop
    pub async fn run(&self) {
        let interval = {
            let config = self.state.config.read().await;
            config.global.health_check_interval
        };

        info!("Health monitor started with interval: {}s", interval);
        let mut shutdown_rx = self.state.shutdown_tx.subscribe();

        loop {
            tokio::select! {
                _ = sleep(Duration::from_secs(interval)) => {
                    self.perform_checks().await;
                }
                _ = shutdown_rx.recv() => {
                    info!("Health monitor shutting down");
                    break;
                }
            }
        }
    }

    /// Perform all health checks
    async fn perform_checks(&self) {
        debug!("Performing health checks");

        // Check for heartbeat file
        self.check_heartbeat().await;

        // Check for overdue jobs
        self.check_overdue_jobs().await;

        // Check for zombie jobs (running but not in state)
        self.check_zombie_jobs().await;

        // Check database connectivity
        self.check_database().await;
    }

    /// Write to heartbeat file if configured
    async fn check_heartbeat(&self) {
        let path = {
            let config = self.state.config.read().await;
            config.global.heartbeat_path.clone()
        };

        if let Some(path) = path {
            let timestamp = chrono::Utc::now().to_rfc3339();
            if let Err(e) = tokio::fs::write(&path, timestamp).await {
                error!("Failed to write heartbeat file at {:?}: {}", path, e);
            }
        }
    }

    /// Check for jobs that have missed their schedule
    async fn check_overdue_jobs(&self) {
        let jobs = {
            let config = self.state.config.read().await;
            config.jobs.iter().map(|j| j.name.clone()).collect::<Vec<_>>()
        };

        let scheduler = self.state.scheduler.read().await;
        for job_name in jobs {
            if scheduler.missed_runs_count(&job_name) > 0 {
                warn!("Job '{}' is overdue", job_name);
                // Drop the lock before awaiting
                drop(scheduler);
                self.send_missed_notification(&job_name).await;
                // Re-acquire lock for next iteration
                return; // Simple strategy: process one at a time per check cycle to avoid lock issues
            }
        }
    }

    /// Check for jobs that are running but not tracked in state
    async fn check_zombie_jobs(&self) {
        // This would require OS-level process inspection, which is complex.
        // For now, we'll rely on the internal state being correct.
        // A more advanced implementation could check for child processes.
    }

    /// Check database connectivity
    async fn check_database(&self) {
        if let Err(e) = self.state.db.connect().await {
            error!("Database health check failed: {}", e);
        }
    }

    /// Send a notification for a missed job
    async fn send_missed_notification(&self, job_name: &str) {
        use crate::notifications::{self, NotificationEvent};

        let config = self.state.config.read().await;
        if let Some(job) = config.jobs.iter().find(|j| j.name == job_name) {
            let threshold = match job.notifications.missed_threshold_minutes {
                Some(value) if value > 0 => value,
                _ => return,
            };

            let mut last_run_str = None;

            let mut job_state_map = self.state.job_state.write().await;
            if let Some(job_state) = job_state_map.get_mut(job_name) {
                let now = chrono::Utc::now();
                if job_state
                    .last_missed_alert
                    .is_some_and(|last| now - last < chrono::Duration::minutes(threshold as i64))
                {
                    return;
                }
                job_state.record_missed_alert(now);
                last_run_str = job_state.last_run.map(|d| d.to_rfc3339());
                let _ = self.state.persist_job_state(job_name, job_state).await;
            }
            drop(job_state_map); // Release lock

            let expected = {
                let scheduler = self.state.scheduler.read().await;
                scheduler.next_expected_run(job_name)
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_else(|| "unknown".to_string())
            };

            if let Some(dispatcher) = notifications::build_dispatcher(&job.notifications) {
                if let Ok(payload) = notifications::NotificationPayload::new(
                    NotificationEvent::JobMissed {
                        job: job_name.to_string(),
                        expected,
                        last_run: last_run_str,
                    },
                ) {
                    dispatcher.send(payload, &job.retry).await;
                }
            }
        }
    }
}
