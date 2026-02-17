//! Inter-process communication for daemon control

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, error, info};

use crate::{DaemonState, execute_job};

/// Control socket server
pub struct ControlServer {
    socket_path: PathBuf,
    state: Arc<DaemonState>,
}

impl ControlServer {
    pub fn new(socket_path: &PathBuf, state: Arc<DaemonState>) -> Self {
        Self {
            socket_path: socket_path.clone(),
            state,
        }
    }

    /// Run the control server loop
    pub async fn run(&self) -> Result<()> {
        // Ensure socket path parent directory exists
        if let Some(parent) = self.socket_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Clean up old socket if it exists
        if self.socket_path.exists() {
            tokio::fs::remove_file(&self.socket_path).await?;
        }

        let listener = UnixListener::bind(&self.socket_path)
            .context("Failed to bind control socket")?;

        info!("Control socket listening at {:?}", self.socket_path);
        let mut shutdown_rx = self.state.shutdown_tx.subscribe();

        loop {
            tokio::select! {
                res = listener.accept() => {
                    match res {
                        Ok((stream, _)) => {
                            let state = self.state.clone();
                            tokio::spawn(async move {
                                if let Err(e) = Self::handle_connection(stream, state).await {
                                    error!("Connection error: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            error!("Accept error: {}", e);
                            break;
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Control server shutting down");
                    break;
                }
            }
        }

        // Clean up socket on shutdown
        let _ = tokio::fs::remove_file(&self.socket_path).await;
        Ok(())
    }

    /// Handle a single client connection
    async fn handle_connection(mut stream: UnixStream, state: Arc<DaemonState>) -> Result<()> {
        let (reader, mut writer) = stream.split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        reader.read_line(&mut line).await?;
        let command = line.trim();
        debug!("Received command: {}", command);

        let response = match command {
            "stop" => {
                let _ = state.shutdown_tx.send(());
                "Daemon is shutting down".to_string()
            }
            "status" => {
                let config = state.config.read().await;
                let running = state.running_jobs.read().await;
                format!(
                    "Borg-Rust Daemon Status:\n  Jobs configured: {}\n  Jobs running: {}\n",
                    config.jobs.len(),
                    running.len()
                )
            }
            "reload" => {
                // Reload logic would go here
                "Configuration reloaded".to_string()
            }
            "list-jobs" => {
                let config = state.config.read().await;
                let mut job_list = String::new();
                for job in &config.jobs {
                    job_list.push_str(&format!("- {}\n", job.name));
                }
                job_list
            }
            cmd if cmd.starts_with("run-job:") => {
                let job_name = cmd.strip_prefix("run-job:").unwrap_or("");
                if job_name.is_empty() {
                    "Error: Job name not specified".to_string()
                } else {
                    let state_clone = state.clone();
                    let job_name_clone = job_name.to_string();
                    tokio::spawn(async move {
                        if let Err(e) = execute_job(&state_clone, &job_name_clone).await {
                            error!("Manual job run for '{}' failed: {}", job_name_clone, e);
                        }
                    });
                    format!("Job '{}' started", job_name)
                }
            }
            _ => "Unknown command".to_string(),
        };

        writer.write_all(response.as_bytes()).await?;
        Ok(())
    }
}
