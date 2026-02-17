//! Borg-Rust Daemon (borgd)
//!
//! A Linux daemon for scheduled backup operations with systemd integration.

mod config;
mod health;
mod ipc;
mod job;
mod notifications;
mod scheduler;
mod state;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use borg_core::credentials;
use borg_core::db::{Database, JobExecutionHistory, JobState as DbJobState};
use config::DaemonConfig;
use health::HealthMonitor;
use ipc::ControlServer;
use scheduler::Scheduler;
use state::JobState;

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
    /// Test notification configuration
    NotifyTest {
        /// Job name to use for notification settings
        name: String,
    },
    /// Validate configuration file
    Validate,
}

/// Shared daemon state
pub struct DaemonState {
    config: RwLock<DaemonConfig>,
    scheduler: RwLock<Scheduler>,
    running_jobs: RwLock<Vec<String>>,
    job_state: RwLock<std::collections::HashMap<String, JobState>>,
    shutdown_tx: broadcast::Sender<()>,
    db: Arc<Database>,
}

impl DaemonState {
    fn new(config: DaemonConfig, shutdown_tx: broadcast::Sender<()>, db: Arc<Database>) -> Self {
        let mut job_state = std::collections::HashMap::new();
        for job in &config.jobs {
            job_state.insert(job.name.clone(), JobState::default());
        }
        Self {
            config: RwLock::new(config.clone()),
            scheduler: RwLock::new(Scheduler::new(config.jobs.clone())),
            running_jobs: RwLock::new(Vec::new()),
            job_state: RwLock::new(job_state),
            shutdown_tx,
            db,
        }
    }

    pub async fn load_job_states(&self) -> Result<()> {
        let mut job_states = self.job_state.write().await;
        for job_name in job_states.keys().cloned().collect::<Vec<_>>() {
            if let Ok(Some(db_state)) = self.db.get_job_state(&job_name).await {
                // Convert DbJobState to JobState
                let state = JobState {
                    last_run: db_state.last_run.and_then(|s| s.parse().ok()),
                    last_success: db_state.last_success.and_then(|s| s.parse().ok()),
                    last_failure: db_state.last_failure.and_then(|s| s.parse().ok()),
                    consecutive_failures: db_state.consecutive_failures as u32,
                    last_error: db_state.last_error,
                    last_failure_alert: db_state.last_failure_alert.and_then(|s| s.parse().ok()),
                    last_missed_alert: db_state.last_missed_alert.and_then(|s| s.parse().ok()),
                };
                job_states.insert(job_name, state);
            }
        }
        Ok(())
    }

    pub async fn persist_job_state(&self, job_name: &str, state: &JobState) -> Result<()> {
        let db_state = DbJobState {
            job_name: job_name.to_string(),
            last_run: state.last_run.map(|d| d.to_rfc3339()),
            last_success: state.last_success.map(|d| d.to_rfc3339()),
            last_failure: state.last_failure.map(|d| d.to_rfc3339()),
            consecutive_failures: state.consecutive_failures as i32,
            last_error: state.last_error.clone(),
            last_failure_alert: state.last_failure_alert.map(|d| d.to_rfc3339()),
            last_missed_alert: state.last_missed_alert.map(|d| d.to_rfc3339()),
        };
        self.db.upsert_job_state(&db_state).await?;
        Ok(())
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
        Some(Commands::NotifyTest { name }) => {
            return send_control_command(&cli.socket, &format!("notify-test:{}", name)).await;
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

    info!(
        "Loaded {} backup jobs from configuration",
        config.jobs.len()
    );

    // Daemonize if not running in foreground
    if !cli.foreground {
        daemonize(&cli.pid_file)?;
    }

    // Initialize database
    let db_path = &config.global.database_path;
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).context("Failed to create database directory")?;
    }
    let db = Arc::new(
        Database::new(db_path)
            .await
            .context("Failed to initialize database")?,
    );

    // Create shutdown channel
    let (shutdown_tx, _) = broadcast::channel(1);

    // Create shared state
    let state = Arc::new(DaemonState::new(config, shutdown_tx.clone(), db));

    // Load persisted job states
    if let Err(e) = state.load_job_states().await {
        warn!("Failed to load job states from database: {}", e);
    }

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
    #[cfg(unix)]
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
    #[cfg(unix)]
    {
        let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Stopping]);
    }

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

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    if foreground {
        // Log to stderr when running in foreground
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().with_writer(std::io::stderr))
            .init();
    } else {
        // Log to journald when running as daemon
        let journald_layer = tracing_journald::layer().context("Failed to connect to journald")?;

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

    let mut sigterm = signal(SignalKind::terminate()).expect("Failed to register SIGTERM handler");

    sigterm.recv().await;
}

/// Run the backup scheduler
async fn run_scheduler(state: Arc<DaemonState>) {
    let mut shutdown_rx = state.shutdown_tx.subscribe();
    let _last_check = chrono::Utc::now();

    loop {
        // Check every 30 seconds for pending jobs or updates
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                let now = chrono::Utc::now();

                // Check if configuration has been reloaded
                // (This would be set by the control server when reload is called)

                // Get next job
                let next_job = {
                    let scheduler = state.scheduler.read().await;
                    scheduler.next_job()
                };

                if let Some((job_name, run_time)) = next_job {
                    let delay = run_time - now;

                    // If the job should have run or is very close (within 60 seconds), execute it
                    if delay.num_seconds() <= 0 {
                        // Execute job and reschedule
                        info!("Executing scheduled job: {}", job_name);
                        if let Err(e) = execute_job(&state, job_name.as_str()).await {
                            error!("Job '{}' failed: {}", job_name, e);
                            // Mark as failed in scheduler for missed run tracking
                            {
                                let mut scheduler = state.scheduler.write().await;
                                scheduler.job_failed(&job_name);
                            }
                        } else {
                            // Reschedule the job
                            let mut scheduler = state.scheduler.write().await;
                            scheduler.job_completed(&job_name);
                        }
                    } else if delay.num_seconds() <= 60 {
                        // Job is coming up soon, sleep until it's time
                        let delay_duration = delay.to_std().unwrap_or(std::time::Duration::ZERO);
                        tokio::select! {
                            _ = tokio::time::sleep(delay_duration) => {
                                info!("Executing scheduled job: {}", job_name);
                                if let Err(e) = execute_job(&state, job_name.as_str()).await {
                                    error!("Job '{}' failed: {}", job_name, e);
                                    let mut scheduler = state.scheduler.write().await;
                                    scheduler.job_failed(&job_name);
                                } else {
                                    let mut scheduler = state.scheduler.write().await;
                                    scheduler.job_completed(&job_name);
                                }
                            }
                            _ = shutdown_rx.recv() => {
                                break;
                            }
                        }
                    }
                    // If job is more than 60 seconds away, loop again and check after 30 seconds
                } else {
                    debug!("No jobs scheduled");
                }
            }
            _ = shutdown_rx.recv() => {
                info!("Scheduler shutting down");
                break;
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
        config
            .jobs
            .iter()
            .find(|j| j.name == job_name)
            .cloned()
            .context("Job not found")?
    };

    // Get password from keyring
    let password = {
        let repos = state.db.list_repos().await?;
        let repo_bookmark = repos.iter().find(|r| r.path == job_config.repository);

        if let Some(bookmark) = repo_bookmark {
            if let Ok(uuid) = Uuid::parse_str(&bookmark.uuid) {
                match credentials::get_password(&uuid) {
                    Ok(pass) => Some(pass),
                    Err(e) => {
                        warn!("Failed to get password from keyring for repo {}: {}", bookmark.name, e);
                        None
                    }
                }
            } else {
                warn!("Invalid UUID for repo {}", bookmark.name);
                None
            }
        } else {
            warn!("No repository bookmark found for path: {}", job_config.repository);
            None
        }
    };

    if password.is_none() {
        warn!("No repository password found on Keyring for job '{}'.", job_name);
    }

    let start_time = chrono::Utc::now();
    // Execute backup
    let result = job::run_backup_job(&job_config, password).await;
    let end_time = chrono::Utc::now();

    // Remove from running jobs
    {
        let mut running = state.running_jobs.write().await;
        running.retain(|j| j != job_name);
    }

    match &result {
        Ok(stats) => {
            info!(
                "Job '{}' completed successfully: {} files, {} bytes",
                job_name, stats.files_processed, stats.bytes_processed
            );

            // Record in database history
            let _ = state
                .db
                .record_job_execution(&JobExecutionHistory {
                    id: None,
                    job_name: job_name.to_string(),
                    status: "success".to_string(),
                    start_time: start_time.to_rfc3339(),
                    end_time: Some(end_time.to_rfc3339()),
                    files_processed: stats.files_processed as i64,
                    bytes_processed: stats.bytes_processed as i64,
                    archive_name: Some(stats.archive_name.clone()),
                    error_message: None,
                })
                .await;

            update_job_state_success(state, job_name, end_time).await;
            send_success_notification(state, job_name, Some(stats.archive_name.clone())).await;
            mark_job_completed(state, job_name).await;
        }
        Err(e) => {
            error!("Job '{}' failed: {}", job_name, e);

            // Record in database history
            let _ = state
                .db
                .record_job_execution(&JobExecutionHistory {
                    id: None,
                    job_name: job_name.to_string(),
                    status: "failure".to_string(),
                    start_time: start_time.to_rfc3339(),
                    end_time: Some(end_time.to_rfc3339()),
                    files_processed: 0,
                    bytes_processed: 0,
                    archive_name: None,
                    error_message: Some(e.to_string()),
                })
                .await;

            update_job_state_failure(state, job_name, end_time, e.to_string()).await;
            send_failure_notification(state, job_name, e.to_string()).await;
            send_failure_threshold_notification(state, job_name).await;
            mark_job_completed(state, job_name).await;
        }
    }

    result.map(|_| ())
}

async fn update_job_state_success(
    daemon_state: &Arc<DaemonState>,
    job_name: &str,
    now: chrono::DateTime<chrono::Utc>,
) {
    let mut job_state_map = daemon_state.job_state.write().await;
    if let Some(job_state) = job_state_map.get_mut(job_name) {
        job_state.record_success(now);
        let _ = daemon_state.persist_job_state(job_name, job_state).await;
    }
}

async fn update_job_state_failure(
    daemon_state: &Arc<DaemonState>,
    job_name: &str,
    now: chrono::DateTime<chrono::Utc>,
    error: String,
) {
    let mut job_state_map = daemon_state.job_state.write().await;
    if let Some(job_state) = job_state_map.get_mut(job_name) {
        job_state.record_failure(now, error);
        let _ = daemon_state.persist_job_state(job_name, job_state).await;
    }
}

async fn mark_job_completed(daemon_state: &Arc<DaemonState>, job_name: &str) {
    let mut scheduler = daemon_state.scheduler.write().await;
    scheduler.job_completed(job_name);
}

async fn send_success_notification(
    daemon_state: &Arc<DaemonState>,
    job_name: &str,
    archive: Option<String>,
) {
    let config = daemon_state.config.read().await;
    if let Some(job) = config.jobs.iter().find(|job| job.name == job_name) {
        if !job.notifications.on_success {
            return;
        }
        if let Some(dispatcher) = notifications::build_dispatcher(&job.notifications) {
            if let Ok(payload) = notifications::NotificationPayload::new(
                notifications::NotificationEvent::JobSuccess {
                    job: job_name.to_string(),
                    archive,
                },
            ) {
                dispatcher.send(payload, &job.retry).await;
            }
        }
    }
}

async fn send_failure_notification(daemon_state: &Arc<DaemonState>, job_name: &str, error: String) {
    let config = daemon_state.config.read().await;
    if let Some(job) = config.jobs.iter().find(|job| job.name == job_name) {
        if !job.notifications.on_failure {
            return;
        }
        if let Some(dispatcher) = notifications::build_dispatcher(&job.notifications) {
            if let Ok(payload) = notifications::NotificationPayload::new(
                notifications::NotificationEvent::JobFailure {
                    job: job_name.to_string(),
                    error,
                },
            ) {
                dispatcher.send(payload, &job.retry).await;
            }
        }
    }
}

async fn send_failure_threshold_notification(daemon_state: &Arc<DaemonState>, job_name: &str) {
    let config = daemon_state.config.read().await;
    if let Some(job) = config.jobs.iter().find(|job| job.name == job_name) {
        let threshold = match job.notifications.failure_threshold {
            Some(value) if value > 0 => value,
            _ => return,
        };

        let mut job_state_map = daemon_state.job_state.write().await;
        if let Some(job_state) = job_state_map.get_mut(job_name) {
            if job_state.consecutive_failures < threshold {
                return;
            }

            let now = chrono::Utc::now();
            if job_state
                .last_failure_alert
                .is_some_and(|last| now - last < chrono::Duration::minutes(5))
            {
                return;
            }
            job_state.record_failure_alert(now);
            let _ = daemon_state.persist_job_state(job_name, job_state).await;
        }

        if let Some(dispatcher) = notifications::build_dispatcher(&job.notifications) {
            if let Ok(payload) = notifications::NotificationPayload::new(
                notifications::NotificationEvent::JobFailure {
                    job: job_name.to_string(),
                    error: format!("Failure threshold exceeded ({threshold})"),
                },
            ) {
                dispatcher.send(payload, &job.retry).await;
            }
        }
    }
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
