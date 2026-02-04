//! Health monitoring and metrics

use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, info};

use crate::DaemonState;

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

        // Check running jobs
        let running_jobs = self.state.running_jobs.read().await;
        if !running_jobs.is_empty() {
            debug!("Running jobs: {:?}", *running_jobs);
        }

        // Notify systemd watchdog if configured
        let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog]);

        // Check repository connectivity for scheduled jobs
        // (In a real implementation, we'd check repositories periodically)

        debug!("Health check completed");
    }
}

/// Metrics collector for Prometheus
pub struct MetricsCollector {
    state: Arc<DaemonState>,
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

        let mut output = String::new();

        // Running jobs gauge
        output.push_str("# HELP borgd_running_jobs_total Number of currently running backup jobs\n");
        output.push_str("# TYPE borgd_running_jobs_total gauge\n");
        output.push_str(&format!("borgd_running_jobs_total {}\n\n", running.len()));

        // Scheduled jobs gauge
        output.push_str("# HELP borgd_scheduled_jobs_total Number of scheduled backup jobs\n");
        output.push_str("# TYPE borgd_scheduled_jobs_total gauge\n");
        output.push_str(&format!("borgd_scheduled_jobs_total {}\n\n", scheduled.len()));

        // Per-job metrics
        output.push_str("# HELP borgd_job_enabled Whether a backup job is enabled\n");
        output.push_str("# TYPE borgd_job_enabled gauge\n");
        for (name, _, enabled) in &scheduled {
            let value = if *enabled { 1 } else { 0 };
            output.push_str(&format!("borgd_job_enabled{{job=\"{}\"}} {}\n", name, value));
        }

        output
    }
}
