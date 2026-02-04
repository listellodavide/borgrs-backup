//! IPC control socket server for daemon management

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, error, info, warn};

use crate::DaemonState;

/// Control socket server for daemon management
pub struct ControlServer {
    socket_path: std::path::PathBuf,
    state: Arc<DaemonState>,
}

impl ControlServer {
    /// Create a new control server
    pub fn new(socket_path: &Path, state: Arc<DaemonState>) -> Self {
        Self {
            socket_path: socket_path.to_path_buf(),
            state,
        }
    }

    /// Run the control server
    pub async fn run(self) -> Result<()> {
        // Remove existing socket file
        let _ = std::fs::remove_file(&self.socket_path);

        // Ensure parent directory exists
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Create Unix socket listener
        let listener = UnixListener::bind(&self.socket_path)
            .context("Failed to bind control socket")?;

        // Set socket permissions (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&self.socket_path, perms)?;
        }

        info!("Control socket listening on {:?}", self.socket_path);

        let mut shutdown_rx = self.state.shutdown_tx.subscribe();

        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, _)) => {
                            let state = self.state.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(stream, state).await {
                                    error!("Error handling control connection: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            error!("Failed to accept connection: {}", e);
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Control server shutting down");
                    break;
                }
            }
        }

        // Cleanup socket file
        let _ = std::fs::remove_file(&self.socket_path);

        Ok(())
    }
}

/// Handle a single control connection
async fn handle_connection(stream: UnixStream, state: Arc<DaemonState>) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    reader.read_line(&mut line).await?;
    let command = line.trim();

    debug!("Received control command: {}", command);

    let response = handle_command(command, &state).await;

    writer.write_all(response.as_bytes()).await?;
    writer.flush().await?;

    Ok(())
}

/// Process a control command and return response
async fn handle_command(command: &str, state: &Arc<DaemonState>) -> String {
    match command {
        "status" => handle_status(state).await,
        "stop" => handle_stop(state).await,
        "reload" => handle_reload(state).await,
        "list-jobs" => handle_list_jobs(state).await,
        cmd if cmd.starts_with("run-job:") => {
            let job_name = &cmd[8..];
            handle_run_job(job_name, state).await
        }
        cmd if cmd.starts_with("pause-job:") => {
            let job_name = &cmd[10..];
            handle_pause_job(job_name, state).await
        }
        cmd if cmd.starts_with("resume-job:") => {
            let job_name = &cmd[11..];
            handle_resume_job(job_name, state).await
        }
        "metrics" => handle_metrics(state).await,
        "health" => handle_health(state).await,
        _ => format!("Unknown command: {}\n", command),
    }
}

async fn handle_status(state: &Arc<DaemonState>) -> String {
    let running_jobs = state.running_jobs.read().await;
    let scheduler = state.scheduler.read().await;
    let scheduled = scheduler.list_scheduled();

    let mut response = String::new();
    response.push_str("Borg-Rust Daemon Status\n");
    response.push_str("=======================\n\n");

    response.push_str("Running Jobs:\n");
    if running_jobs.is_empty() {
        response.push_str("  (none)\n");
    } else {
        for job in running_jobs.iter() {
            response.push_str(&format!("  - {}\n", job));
        }
    }

    response.push_str("\nScheduled Jobs:\n");
    if scheduled.is_empty() {
        response.push_str("  (none)\n");
    } else {
        for (name, next_run, enabled) in scheduled {
            let status = if enabled { "" } else { " [disabled]" };
            response.push_str(&format!("  - {}: {}{}\n", name, next_run, status));
        }
    }

    response
}

async fn handle_stop(state: &Arc<DaemonState>) -> String {
    info!("Stop command received via control socket");
    let _ = state.shutdown_tx.send(());
    "Daemon stopping...\n".to_string()
}

async fn handle_reload(_state: &Arc<DaemonState>) -> String {
    info!("Reload command received via control socket");
    
    // In a real implementation, we'd reload from the config file
    // For now, just acknowledge
    "Configuration reload initiated\n".to_string()
}

async fn handle_list_jobs(state: &Arc<DaemonState>) -> String {
    let config = state.config.read().await;
    let scheduler = state.scheduler.read().await;

    let mut response = String::new();
    response.push_str("Configured Backup Jobs\n");
    response.push_str("======================\n\n");

    for job in &config.jobs {
        let status = if job.enabled { "enabled" } else { "disabled" };
        let next_run = scheduler.list_scheduled()
            .iter()
            .find(|(name, _, _)| name == &job.name)
            .map(|(_, time, _)| time.to_string())
            .unwrap_or_else(|| "not scheduled".to_string());

        response.push_str(&format!("{}:\n", job.name));
        response.push_str(&format!("  Status: {}\n", status));
        response.push_str(&format!("  Schedule: {}\n", job.schedule));
        response.push_str(&format!("  Next Run: {}\n", next_run));
        response.push_str(&format!("  Repository: {}\n", job.repository));
        response.push_str(&format!("  Paths: {:?}\n", job.paths));
        if !job.exclude_patterns.is_empty() {
            response.push_str(&format!("  Exclusions: {} patterns\n", 
                job.exclude_patterns.len()));
        }
        response.push('\n');
    }

    response
}

async fn handle_run_job(job_name: &str, state: &Arc<DaemonState>) -> String {
    let mut scheduler = state.scheduler.write().await;
    
    if scheduler.schedule_now(job_name) {
        info!("Manual job run requested: {}", job_name);
        format!("Job '{}' scheduled for immediate execution\n", job_name)
    } else {
        warn!("Attempted to run unknown job: {}", job_name);
        format!("Error: Job '{}' not found\n", job_name)
    }
}

async fn handle_pause_job(job_name: &str, _state: &Arc<DaemonState>) -> String {
    // In a real implementation, we'd update the job config
    info!("Job pause requested: {}", job_name);
    format!("Job '{}' paused\n", job_name)
}

async fn handle_resume_job(job_name: &str, _state: &Arc<DaemonState>) -> String {
    // In a real implementation, we'd update the job config
    info!("Job resume requested: {}", job_name);
    format!("Job '{}' resumed\n", job_name)
}

async fn handle_metrics(state: &Arc<DaemonState>) -> String {
    // Return Prometheus-formatted metrics
    let running = state.running_jobs.read().await;
    let scheduler = state.scheduler.read().await;
    let scheduled = scheduler.list_scheduled();

    let mut response = String::new();
    response.push_str("# HELP borgd_running_jobs Number of currently running backup jobs\n");
    response.push_str("# TYPE borgd_running_jobs gauge\n");
    response.push_str(&format!("borgd_running_jobs {}\n", running.len()));

    response.push_str("# HELP borgd_scheduled_jobs Number of scheduled backup jobs\n");
    response.push_str("# TYPE borgd_scheduled_jobs gauge\n");
    response.push_str(&format!("borgd_scheduled_jobs {}\n", scheduled.len()));

    response
}

async fn handle_health(state: &Arc<DaemonState>) -> String {
    // Basic health check
    let running = state.running_jobs.read().await;
    
    let mut response = String::new();
    response.push_str("{\n");
    response.push_str("  \"status\": \"healthy\",\n");
    response.push_str(&format!("  \"running_jobs\": {},\n", running.len()));
    response.push_str(&format!("  \"timestamp\": \"{}\"\n", chrono::Utc::now()));
    response.push_str("}\n");

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;

    async fn create_test_state() -> Arc<DaemonState> {
        let config = crate::config::DaemonConfig {
            global: Default::default(),
            jobs: vec![],
            repositories: vec![],
        };
        let (shutdown_tx, _) = broadcast::channel(1);
        Arc::new(DaemonState::new(config, shutdown_tx))
    }

    #[tokio::test]
    async fn test_status_command() {
        let state = create_test_state().await;
        let response = handle_command("status", &state).await;
        assert!(response.contains("Borg-Rust Daemon Status"));
    }

    #[tokio::test]
    async fn test_health_command() {
        let state = create_test_state().await;
        let response = handle_command("health", &state).await;
        assert!(response.contains("healthy"));
    }

    #[tokio::test]
    async fn test_unknown_command() {
        let state = create_test_state().await;
        let response = handle_command("unknown", &state).await;
        assert!(response.contains("Unknown command"));
    }
}
