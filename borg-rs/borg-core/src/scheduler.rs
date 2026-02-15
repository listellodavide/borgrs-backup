use async_trait::async_trait;

use cronscheduler::{
    ExecutionPolicy, ReactiveTask, SchedulerActor, SchedulingPolicy, TaskContext, TaskType,
    WorkerActor,
};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum SchedulerCommand {
    AddTask(ScheduledTask),
    RemoveTask(String), // task name
    UpdateTask(ScheduledTask),
    Shutdown,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum ScheduleType {
    Daily,
    Weekly,
    Monthly,
    Manual,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackupSchedule {
    pub schedule_type: ScheduleType,
    pub weekday: u32,      // 0 = Mon … 6 = Sun
    pub day_of_month: u32, // 1-31 for monthly
    pub hour: u32,
    pub minute: u32,
    pub run_on_boot_if_missed: bool,
}

impl BackupSchedule {
    pub fn to_cron_string(&self) -> String {
        match self.schedule_type {
            ScheduleType::Daily => {
                format!("0 {} {} * * *", self.minute, self.hour)
            }
            ScheduleType::Weekly => {
                // cronscheduler likely expects 0-6 (Sun-Sat) or 1-7 (Mon-Sun)
                // Existing code: 0 = Mon ... 6 = Sun
                // Usually 0 is Sunday in cron.
                // Converting 0-6 (Mon-Sun) to cron DOW:
                // Mon (0) -> 1, Tue (1) -> 2, ..., Sun (6) -> 0
                let cron_dow = if self.weekday == 6 {
                    0
                } else {
                    self.weekday + 1
                };
                format!("0 {} {} * * {}", self.minute, self.hour, cron_dow)
            }
            ScheduleType::Monthly => {
                format!("0 {} {} {} * *", self.minute, self.hour, self.day_of_month)
            }
            ScheduleType::Manual => {
                // Manual tasks shouldn't be in the cronscheduler, or they can be scheduled in the far future/never.
                // However, cronscheduler might not like invalid strings.
                // We'll return a special string or handle it in the caller.
                "0 0 0 1 1 ? 2099".to_string() // Far future
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScheduledTask {
    pub task_name: String,
    pub repo_name: String,
    pub repo_path: String,
    pub archive_name: String,
    pub paths_to_backup: Vec<String>,
    pub compression: String,
    pub comment: Option<String>,
    pub tags: Option<Vec<String>>,
    pub schedule: BackupSchedule,
    pub last_run: Option<SystemTime>,
    pub execution_count: u32,
}

#[async_trait]
pub trait SchedulerReporter: Send + Sync + std::fmt::Debug {
    fn on_task_start(&self, task: &ScheduledTask);
    fn on_task_progress(&self, task: &ScheduledTask, processed: u64, total: u64);
    fn on_task_complete(&self, task: &ScheduledTask, summary: String);
    fn on_task_error(&self, task: &ScheduledTask, error: String);
    async fn get_password(
        &self,
        _repo_path: &str,
        _repo_name: &str,
        _retry_count: usize,
    ) -> Option<String> {
        None
    }
    fn on_file_processed(&self, _task: &ScheduledTask, _filename: &str) {}
    fn on_scheduler_tick(&self, _tasks_found: usize) {}
}

#[derive(Debug)]
pub struct BorgBackupTask {
    pub task_data: ScheduledTask,
    pub reporter: Option<Arc<dyn SchedulerReporter>>,
}

#[async_trait]
impl ReactiveTask for BorgBackupTask {
    fn id(&self) -> &str {
        &self.task_data.task_name
    }

    fn task_type(&self) -> TaskType {
        TaskType::Blocking
    }

    async fn execute(
        &self,
        _context: TaskContext,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let task = &self.task_data;
        if let Some(reporter) = &self.reporter {
            reporter.on_task_start(task);
        }

        // Construct archive name with timestamp
        let timestamp = crate::archive::current_time_for_archive_name();
        let archive_name = format!("{}-{}", task.archive_name, timestamp);

        println!("Executing scheduled backup for {}", archive_name);

        // Perform the actual backup with retry logic for password
        let storage = crate::storage::StorageConfig::Local {
            path: std::path::PathBuf::from(&task.repo_path),
        };

        let mut last_error = None;
        for retry in 0..3 {
            let password = if let Some(reporter) = &self.reporter {
                reporter
                    .get_password(&task.repo_path, &task.repo_name, retry)
                    .await
            } else {
                None
            };

            match crate::storage::build_operator(storage.clone()) {
                Ok(op) => {
                    match crate::repository::Repository::open(
                        op,
                        task.repo_path.clone(),
                        password.as_deref(),
                    )
                    .await
                    {
                        Ok(mut repo) => {
                            let paths: Vec<std::path::PathBuf> = task
                                .paths_to_backup
                                .iter()
                                .map(std::path::PathBuf::from)
                                .collect();

                            // Create progress callback
                            let reporter_clone = self.reporter.clone();
                            let task_clone = task.clone();
                            let progress_callback =
                                move |processed: u64, total: u64, current_file: Option<&str>| {
                                    if let Some(r) = &reporter_clone {
                                        r.on_task_progress(&task_clone, processed, total);
                                        if let Some(file) = current_file {
                                            r.on_file_processed(&task_clone, file);
                                        }
                                    }
                                };

                            match repo
                                .create_archive(
                                    &archive_name,
                                    &paths,
                                    &task.compression,
                                    task.comment.as_deref(),
                                    task.tags.as_deref(),
                                    progress_callback,
                                )
                                .await
                            {
                                Ok(summary) => {
                                    if let Some(reporter) = &self.reporter {
                                        reporter.on_task_complete(task, summary);
                                    }
                                    return Ok(());
                                }
                                Err(e) => {
                                    let err_msg = e.to_string();
                                    last_error = Some(err_msg.clone());
                                    // If it's not a password error, don't necessarily retry?
                                    // But Borg open errors usually happen at Repository::open.
                                    // If create_archive fails, it might be something else.
                                    if !err_msg.contains("password")
                                        && !err_msg.contains("passphrase")
                                    {
                                        if let Some(reporter) = &self.reporter {
                                            reporter.on_task_error(task, err_msg.clone());
                                        }
                                        return Err(err_msg.into());
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            let err_msg = format!("Failed to open repository: {}", e);
                            last_error = Some(err_msg.clone());
                            if !err_msg.contains("password") && !err_msg.contains("passphrase") {
                                if let Some(reporter) = &self.reporter {
                                    reporter.on_task_error(task, err_msg.clone());
                                }
                                return Err(err_msg.into());
                            }
                        }
                    }
                }
                Err(e) => {
                    let err_msg = format!("Failed to build storage operator: {}", e);
                    if let Some(reporter) = &self.reporter {
                        reporter.on_task_error(task, err_msg.clone());
                    }
                    return Err(err_msg.into());
                }
            }
        }

        let final_err = last_error.unwrap_or_else(|| "Failed after 3 attempts".to_string());
        if let Some(reporter) = &self.reporter {
            reporter.on_task_error(task, final_err.clone());
        }
        Err(final_err.into())
    }
}

// Global command channel sender (initialized at scheduler startup)
static SCHEDULER_CMD_TX: Mutex<Option<mpsc::UnboundedSender<SchedulerCommand>>> = Mutex::new(None);

// Global flag to trigger scheduler reload
static SCHEDULER_RELOAD_FLAG: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

// Global callback to reload tasks from external source (e.g., app_state)
static SCHEDULER_TASK_RELOADER: Mutex<
    Option<Arc<dyn Fn() -> Vec<(ScheduledTask, String)> + Send + Sync>>,
> = Mutex::new(None);

pub struct Scheduler {
    tasks: Mutex<Vec<(ScheduledTask, String)>>, // task and its cron string
    reporter: Option<Arc<dyn SchedulerReporter>>,
}

impl Scheduler {
    pub fn new(tasks_with_cron: Vec<(ScheduledTask, String)>) -> Self {
        Self {
            tasks: Mutex::new(tasks_with_cron),
            reporter: None,
        }
    }

    pub fn with_reporter(mut self, reporter: Arc<dyn SchedulerReporter>) -> Self {
        self.reporter = Some(reporter);
        self
    }

    /// Set a callback to reload tasks from external source (called on reload)
    pub fn set_task_reloader<F>(reloader: F) -> Result<(), String>
    where
        F: Fn() -> Vec<(ScheduledTask, String)> + Send + Sync + 'static,
    {
        let mut callback = SCHEDULER_TASK_RELOADER.lock().map_err(|e| e.to_string())?;
        *callback = Some(Arc::new(reloader));
        Ok(())
    }

    /// Send a command to the running scheduler
    pub fn send_command(cmd: SchedulerCommand) -> Result<(), String> {
        let tx = SCHEDULER_CMD_TX.lock().map_err(|e| e.to_string())?;
        if let Some(tx) = tx.as_ref() {
            tx.send(cmd).map_err(|e| e.to_string())
        } else {
            Err("Scheduler not initialized".to_string())
        }
    }

    /// Request scheduler to reload tasks (used when tasks are updated via GUI)
    pub fn request_reload() {
        SCHEDULER_RELOAD_FLAG.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub async fn run(&self) {
        println!("Scheduler: Starting");

        // Create command channel for signaling
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
        {
            let mut tx_lock = SCHEDULER_CMD_TX.lock().unwrap();
            *tx_lock = Some(cmd_tx);
        }

        // Initial task load
        self.load_and_start_tasks(&mut cmd_rx).await;
    }

    async fn load_and_start_tasks(&self, cmd_rx: &mut mpsc::UnboundedReceiver<SchedulerCommand>) {
        loop {
            // Load current tasks from persistent state (reload from JSON each iteration)
            let (current_tasks, active_count) = {
                let tasks = self.tasks.lock().unwrap();
                let filtered: Vec<_> = tasks
                    .iter()
                    .filter(|(task, _)| task.schedule.schedule_type != ScheduleType::Manual)
                    .collect();
                let count = filtered.len();
                let collected = filtered.into_iter().cloned().collect::<Vec<_>>();
                (collected, count)
            };

            if current_tasks.is_empty() {
                println!("Scheduler: No tasks to schedule, waiting for commands...");
            } else {
                println!("Scheduler: Loading {} tasks", current_tasks.len());
                // Debug: list all loaded tasks
                for (task, cron) in &current_tasks {
                    println!("  - {} ({})", task.archive_name, cron);
                }
            }

            // Create fresh worker and scheduler for this iteration
            let (worker_tx, worker_rx) = mpsc::channel(100);
            let worker = WorkerActor::new(worker_rx);
            tokio::spawn(async move {
                worker.run().await;
            });

            let mut cron_scheduler = SchedulerActor::new(worker_tx);

            // Add all tasks to scheduler
            let mut has_errors = false;
            for (task, cron_str) in current_tasks.iter() {
                let cron_task = Arc::new(BorgBackupTask {
                    task_data: task.clone(),
                    reporter: self.reporter.clone(),
                });

                match cron_scheduler.add_task(
                    cron_task,
                    cron_str,
                    ExecutionPolicy::SkipIfRunning,
                    SchedulingPolicy::FirstInFirstOut,
                    0,
                ) {
                    Ok(_) => {
                        println!(
                            "Scheduler: Registered '{}' ({})",
                            task.archive_name, cron_str
                        );
                    }
                    Err(e) => {
                        eprintln!(
                            "Scheduler ERROR: Failed to add task '{}' with cron '{}': {}",
                            task.archive_name, cron_str, e
                        );
                        has_errors = true;
                    }
                }
            }

            if has_errors {
                eprintln!("Scheduler: Errors occurred. Please check your task configuration.");
            }

            // Report status before starting
            if let Some(reporter) = &self.reporter {
                reporter.on_scheduler_tick(active_count);
            }

            // Start the cron scheduler (this spawns background tasks for each scheduled item)
            let _ = cron_scheduler.start_all().await;
            println!("Scheduler: Tasks activated, monitoring for changes (check every 20s)");

            // Now wait for reload signal or commands
            // Note: start_all() returns immediately after spawning task loops,
            // so we don't need to wait on a handle - the tasks run in the background
            let mut reload_check = tokio::time::interval(std::time::Duration::from_secs(20));

            loop {
                tokio::select! {
                    // Handle incoming commands
                    Some(cmd) = cmd_rx.recv() => {
                        match cmd {
                            SchedulerCommand::AddTask(_) => {
                                println!("Scheduler: Add task command received");
                                SCHEDULER_RELOAD_FLAG.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                            SchedulerCommand::RemoveTask(_) => {
                                println!("Scheduler: Remove task command received");
                                SCHEDULER_RELOAD_FLAG.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                            SchedulerCommand::UpdateTask(_) => {
                                println!("Scheduler: Update task command received");
                                SCHEDULER_RELOAD_FLAG.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                            SchedulerCommand::Shutdown => {
                                println!("Scheduler: Shutdown signal received");
                                return;
                            }
                        }
                    }

                    // Periodically check reload flag
                    _ = reload_check.tick() => {
                        if SCHEDULER_RELOAD_FLAG.load(std::sync::atomic::Ordering::Relaxed) {
                            SCHEDULER_RELOAD_FLAG.store(false, std::sync::atomic::Ordering::Relaxed);
                            println!("Scheduler: Reload flag detected, restarting task scheduler");

                            // Try to reload tasks from external source
                            {
                                let callback = SCHEDULER_TASK_RELOADER.lock().unwrap();
                                if let Some(reloader) = callback.as_ref() {
                                    let new_tasks = reloader();
                                    println!("Scheduler: Reloaded {} tasks from external source", new_tasks.len());
                                    *self.tasks.lock().unwrap() = new_tasks;
                                }
                            }

                            break; // Break inner loop to reload tasks
                        }
                    }
                }
            }

            // Small delay before reloading to avoid rapid cycles
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::Repository;
    use crate::storage::{StorageConfig, build_operator};
    use std::fs;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    #[derive(Debug)]
    struct TestReporter {
        started: Arc<Mutex<bool>>,
        completed: Arc<Mutex<bool>>,
    }

    #[async_trait]
    impl SchedulerReporter for TestReporter {
        fn on_task_start(&self, _task: &ScheduledTask) {
            *self.started.lock().unwrap() = true;
        }
        fn on_task_progress(&self, _task: &ScheduledTask, _processed: u64, _total: u64) {}
        fn on_task_complete(&self, _task: &ScheduledTask, _summary: String) {
            *self.completed.lock().unwrap() = true;
        }
        fn on_task_error(&self, _task: &ScheduledTask, error: String) {
            println!("Task error: {}", error);
        }
        async fn get_password(
            &self,
            _repo_path: &str,
            _repo_name: &str,
            _retry: usize,
        ) -> Option<String> {
            Some("hello".to_string())
        }
        fn on_file_processed(&self, _task: &ScheduledTask, _filename: &str) {}
        fn on_scheduler_tick(&self, _tasks_found: usize) {}
    }

    #[tokio::test]
    async fn test_scheduler_execution() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path().join("adrianbackup");
        let source_dir = temp_dir.path().join("code");

        fs::create_dir_all(&source_dir).unwrap();
        fs::write(source_dir.join("test.txt"), "test content").unwrap();

        // Initialize repository
        let op = build_operator(StorageConfig::Local {
            path: repo_path.clone(),
        })
        .unwrap();
        let _ = Repository::init(
            op,
            repo_path.to_string_lossy().to_string(),
            Some("hello"),
            None,
        )
        .await
        .unwrap();

        let task = ScheduledTask {
            task_name: "test_task".to_string(),
            repo_name: "test_repo".to_string(),
            repo_path: repo_path.to_string_lossy().to_string(),
            archive_name: "test_archive".to_string(),
            paths_to_backup: vec![source_dir.to_string_lossy().to_string()],
            compression: "zstd,3".to_string(),
            comment: Some("backup code".to_string()),
            tags: Some(vec!["code".to_string(), "adrian".to_string()]),
            schedule: BackupSchedule {
                schedule_type: ScheduleType::Daily,
                weekday: 0,
                day_of_month: 0,
                hour: 0,
                minute: 0,
                run_on_boot_if_missed: true,
            },
            last_run: None,
            execution_count: 0,
        };

        let started = Arc::new(Mutex::new(false));
        let completed = Arc::new(Mutex::new(false));
        let reporter = Arc::new(TestReporter {
            started: started.clone(),
            completed: completed.clone(),
        });

        // Test the BorgBackupTask directly as cronscheduler integration is hard to test in a short unit test
        let cron_task = BorgBackupTask {
            task_data: task,
            reporter: Some(reporter),
        };

        let result = cron_task
            .execute(TaskContext {
                scheduled_time: chrono::Utc::now(),
                actual_time: chrono::Utc::now(),
                weight: 0,
                metadata: std::collections::HashMap::new(),
            })
            .await;
        assert!(result.is_ok());

        assert!(*started.lock().unwrap());
        assert!(*completed.lock().unwrap());
    }
}
