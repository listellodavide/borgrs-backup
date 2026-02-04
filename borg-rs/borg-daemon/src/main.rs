//! Borg-Rust Daemon (borgd)
//!
//! A Linux daemon for scheduled backup operations with systemd integration.

mod config;
mod scheduler;
mod ipc;
mod job;
mod health;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tokio::sync::{broadcast, RwLock};
use tracing::{info, error, warn};

use config::DaemonConfig;
use scheduler::Scheduler;
use ipc::ControlServer;
use health::HealthMonitor;

// Wait, I need to see where borg_core is used.

/// Borg-Rust Daemon - Scheduled backup service
#[derive(Parser, Debug)]
#[command(name = "borgd")]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Configuration file path
    #[arg(short, long, default_value = "/etc/borg-rust/borgd.yaml")]
    config: PathBuf,

    /// Run in foreground (don't daemonize)
    #[arg(short, long)]
    foreground: bool,

    /// PID file location
    #[arg(long, default_value = "/run/borgd/borgd.pid")]
    pid_file: PathBuf,

    /// Control socket path
    #[arg(long, default_value = "/run/borgd/borgd.sock")]
    socket: PathBuf,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    log_level: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the daemon
    Start,
    /// Stop the daemon
    Stop,
    /// Reload configuration
    Reload,
    /// Show daemon status
    Status,
    /// Run a backup job immediately
    RunJob {
        /// Job name to run
        name: String,
    },
    /// List configured backup jobs
    ListJobs,
    /// Validate configuration file
    Validate,
}

/// Shared daemon state
pub struct DaemonState {
    config: RwLock<DaemonConfig>,
    scheduler: RwLock<Scheduler>,
    running_jobs: RwLock<Vec<String>>,
    shutdown_tx: broadcast::Sender<()>,
}

impl DaemonState {
    fn new(config: DaemonConfig, shutdown_tx: broadcast::Sender<()>) -> Self {
        Self {
            config: RwLock::new(config.clone()),
            scheduler: RwLock::new(Scheduler::new(config.jobs.clone())),
            running_jobs: RwLock::new(Vec::new()),
            shutdown_tx,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Handle subcommands that don't require full daemon startup
    match &cli.command {
        Some(Commands::Validate) => {
            return validate_config(&cli.config).await;
        }
        Some(Commands::Stop) => {
            return send_control_command(&cli.socket, "stop").await;
        }
        Some(Commands::Status) => {
            return send_control_command(&cli.socket, "status").await;
        }
        Some(Commands::Reload) => {
            return send_control_command(&cli.socket, "reload").await;
        }
        Some(Commands::ListJobs) => {
            return send_control_command(&cli.socket, "list-jobs").await;
        }
        Some(Commands::RunJob { name }) => {
            return send_control_command(&cli.socket, &format!("run-job:{}", name)).await;
        }
        _ => {}
    }

    // Initialize logging
    init_logging(&cli.log_level, cli.foreground)?;

    info!("Borg-Rust Daemon starting...");

    // Load configuration
    let config = DaemonConfig::load(&cli.config)
        .await
        .context("Failed to load configuration")?;

    info!("Loaded {} backup jobs from configuration", config.jobs.len());

    // Daemonize if not running in foreground
    if !cli.foreground {
        daemonize(&cli.pid_file)?;
    }

    // Create shutdown channel
    let (shutdown_tx, _) = broadcast::channel(1);

    // Create shared state
    let state = Arc::new(DaemonState::new(config, shutdown_tx.clone()));

    // Start control socket server
    let control_server = ControlServer::new(&cli.socket, state.clone());
    let control_handle = tokio::spawn(async move {
        if let Err(e) = control_server.run().await {
            error!("Control server error: {}", e);
        }
    });

    // Start health monitor
    let health_monitor = HealthMonitor::new(state.clone());
    let health_handle = tokio::spawn(async move {
        health_monitor.run().await;
    });

    // Start scheduler
    let scheduler_state = state.clone();
    let scheduler_handle = tokio::spawn(async move {
        run_scheduler(scheduler_state).await;
    });

    // Notify systemd that we're ready
    if !cli.foreground {
        let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Ready]);
    }

    info!("Daemon started successfully");

    // Wait for shutdown signal
    let mut shutdown_rx = shutdown_tx.subscribe();
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Received SIGINT, shutting down...");
        }
        _ = wait_for_sigterm() => {
            info!("Received SIGTERM, shutting down...");
        }
        _ = shutdown_rx.recv() => {
            info!("Received shutdown command, shutting down...");
        }
    }

    // Notify systemd we're stopping
    let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Stopping]);

    // Graceful shutdown
    let _ = shutdown_tx.send(());

    // Wait for tasks to complete with timeout
    let shutdown_timeout = tokio::time::Duration::from_secs(30);
    tokio::select! {
        _ = async {
            let _ = control_handle.await;
            let _ = health_handle.await;
            let _ = scheduler_handle.await;
        } => {
            info!("All tasks shut down gracefully");
        }
        _ = tokio::time::sleep(shutdown_timeout) => {
            warn!("Shutdown timeout reached, forcing exit");
        }
    }

    // Cleanup PID file
    if !cli.foreground {
        let _ = std::fs::remove_file(&cli.pid_file);
    }

    info!("Daemon stopped");
    Ok(())
}

/// Initialize logging subsystem
fn init_logging(level: &str, foreground: bool) -> Result<()> {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    if foreground {
        // Log to stderr when running in foreground
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().with_writer(std::io::stderr))
            .init();
    } else {
        // Log to journald when running as daemon
        let journald_layer = tracing_journald::layer()
            .context("Failed to connect to journald")?;
        
        tracing_subscriber::registry()
            .with(filter)
            .with(journald_layer)
            .init();
    }

    Ok(())
}

/// Daemonize the process
fn daemonize(pid_file: &PathBuf) -> Result<()> {
    use daemonize::Daemonize;

    // Ensure parent directory exists
    if let Some(parent) = pid_file.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let daemon = Daemonize::new()
        .pid_file(pid_file)
        .chown_pid_file(true)
        .working_directory("/")
        .umask(0o027);

    daemon.start().context("Failed to daemonize")?;

    Ok(())
}

/// Wait for SIGTERM signal
async fn wait_for_sigterm() {
    use tokio::signal::unix::{signal, SignalKind};
    
    let mut sigterm = signal(SignalKind::terminate())
        .expect("Failed to register SIGTERM handler");
    
    sigterm.recv().await;
}

/// Run the backup scheduler
async fn run_scheduler(state: Arc<DaemonState>) {
    let mut shutdown_rx = state.shutdown_tx.subscribe();
    
    loop {
        // Check for next scheduled job
        let next_job = {
            let scheduler = state.scheduler.read().await;
            scheduler.next_job()
        };

        if let Some((job_name, run_time)) = next_job {
            let delay = run_time - chrono::Utc::now();
            let delay_duration = delay.to_std().unwrap_or(std::time::Duration::ZERO);

            tokio::select! {
                _ = tokio::time::sleep(delay_duration) => {
                    // Execute the job
                    if let Err(e) = execute_job(&state, &job_name).await {
                        error!("Job '{}' failed: {}", job_name, e);
                    }
                }
                _ = shutdown_rx.recv() => {
                    break;
                }
            }
        } else {
            // No jobs scheduled, wait for a bit
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {}
                _ = shutdown_rx.recv() => {
                    break;
                }
            }
        }
    }
}

/// Execute a backup job
async fn execute_job(state: &Arc<DaemonState>, job_name: &str) -> Result<()> {
    info!("Starting backup job: {}", job_name);

    // Mark job as running
    {
        let mut running = state.running_jobs.write().await;
        running.push(job_name.to_string());
    }

    // Get job configuration
    let job_config = {
        let config = state.config.read().await;
        config.jobs.iter()
            .find(|j| j.name == job_name)
            .cloned()
            .context("Job not found")?
    };

    // Execute backup
    let result = job::run_backup_job(&job_config).await;

    // Remove from running jobs
    {
        let mut running = state.running_jobs.write().await;
        running.retain(|j| j != job_name);
    }

    match &result {
        Ok(stats) => {
            info!("Job '{}' completed successfully: {} files, {} bytes", 
                job_name, stats.files_processed, stats.bytes_processed);
        }
        Err(e) => {
            error!("Job '{}' failed: {}", job_name, e);
        }
    }

    result.map(|_| ())
}

/// Validate configuration file
async fn validate_config(path: &PathBuf) -> Result<()> {
    match DaemonConfig::load(path).await {
        Ok(config) => {
            println!("Configuration is valid!");
            println!("  Jobs configured: {}", config.jobs.len());
            for job in &config.jobs {
                println!("    - {}: {} paths", job.name, job.paths.len());
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("Configuration error: {}", e);
            std::process::exit(1);
        }
    }
}

/// Send a command to the running daemon via control socket
async fn send_control_command(socket_path: &PathBuf, command: &str) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    let mut stream = UnixStream::connect(socket_path)
        .await
        .context("Failed to connect to daemon socket. Is the daemon running?")?;

    stream.write_all(command.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await?;

    let mut response = String::new();
    stream.read_to_string(&mut response).await?;

    println!("{}", response);
    Ok(())
}
