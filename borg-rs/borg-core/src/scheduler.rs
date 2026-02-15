use async_trait::async_trait;
use cron::Schedule;
use std::str::FromStr;

use cronscheduler::{
    ExecutionPolicy, ReactiveTask, SchedulerActor, SchedulingPolicy, TaskContext, TaskType,
    WorkerActor,
};
use std::sync::{Arc, Mutex};
use std::collections::HashSet;
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
        // The underlying cron parser used by the scheduler computes times in UTC (DateTime<Utc>).
        // Users enter schedules in local time; to make scheduled tasks run at the expected local time,
        // convert the user-provided local hour/minute/day into the corresponding UTC values and
        // produce a cron expression using those UTC values.
        use chrono::{Datelike, Local, TimeZone, Timelike, Utc, Duration, Weekday};

        match self.schedule_type {
            ScheduleType::Daily => {
                // Create a Local datetime for today at the configured local hour/minute
                let now_local = Local::now();
                let local_dt = Local
                    .with_ymd_and_hms(now_local.year(), now_local.month(), now_local.day(), self.hour as u32, self.minute as u32, 0)
                    .single()
                    .unwrap_or_else(|| now_local);
                let utc_dt = local_dt.with_timezone(&Utc);
                format!("0 {} {} * * *", utc_dt.minute(), utc_dt.hour())
            }
            ScheduleType::Weekly => {
                // Find the next Local date that matches the configured weekday, set the local time,
                // then convert to UTC and use its weekday/hour/minute for the cron expression.
                let target_weekday = match self.weekday {
                    0 => Weekday::Mon,
                    1 => Weekday::Tue,
                    2 => Weekday::Wed,
                    3 => Weekday::Thu,
                    4 => Weekday::Fri,
                    5 => Weekday::Sat,
                    _ => Weekday::Sun,
                };
                let now_local = Local::now();
                let mut candidate = now_local.date_naive();
                let mut days_to_add = (target_weekday.number_from_monday() as i64 - now_local.weekday().number_from_monday() as i64) % 7;
                if days_to_add < 0 {
                    days_to_add += 7;
                }
                candidate = candidate + Duration::days(days_to_add);
                let local_dt = Local
                    .with_ymd_and_hms(candidate.year(), candidate.month(), candidate.day(), self.hour as u32, self.minute as u32, 0)
                    .single()
                    .unwrap_or_else(|| now_local);
                let utc_dt = local_dt.with_timezone(&Utc);
                // Cron day-of-week is 0=Sun..6=Sat (use num_days_from_sunday)
                let cron_dow = utc_dt.weekday().num_days_from_sunday();
                format!("0 {} {} * * {}", utc_dt.minute(), utc_dt.hour(), cron_dow)
            }
            ScheduleType::Monthly => {
                // Construct a Local datetime for the configured day_of_month (or nearest valid day),
                // convert to UTC and use the UTC day/hour/minute in the cron expression.
                let now_local = Local::now();
                let year = now_local.year();
                let month = now_local.month();
                // Clamp day_of_month to valid range for current month
                let mut day: u32 = self.day_of_month;
                if day < 1 {
                    day = 1;
                }
                // compute days in month safely using NaiveDate for the next month
                let next_month = if month == 12 { 1 } else { month + 1 };
                let next_year = if month == 12 { year + 1 } else { year };
                let first_next = chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1)
                    .or_else(|| chrono::NaiveDate::from_ymd_opt(year, month, 1))
                    .unwrap();
                let last_day = (first_next - chrono::Duration::days(1)).day();
                if day > last_day {
                    day = last_day;
                }
                let local_dt = Local
                    .with_ymd_and_hms(year, month, day, self.hour as u32, self.minute as u32, 0)
                    .single()
                    .unwrap_or_else(|| now_local);
                let utc_dt = local_dt.with_timezone(&Utc);
                format!("0 {} {} {} * *", utc_dt.minute(), utc_dt.hour(), utc_dt.day())
            }
            ScheduleType::Manual => {
                // Manual tasks shouldn't be in the cronscheduler; schedule far future
                "0 0 0 1 1 ? 2099".to_string()
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
        // Keep a set of previous task names so we can detect newly added tasks on reload.
        // If a task with the same name already existed, treat it as an update and do NOT
        // consider it "new" for the missed-run immediate execution.
        let mut prev_names: HashSet<String> = HashSet::new();

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
                        // Compute and print next run time for debugging
                        let next_run_info = match Schedule::from_str(cron_str) {
                            Ok(schedule) => {
                                let now: chrono::DateTime<chrono::Utc> = chrono::Utc::now();
                                let next_dt: Option<chrono::DateTime<chrono::Utc>> = schedule.after(&now).next();
                                match next_dt {
                                    Some(dt) => format!("next run at UTC {}", dt),
                                    None => "no next run found".to_string(),
                                }
                            }
                            Err(e) => format!("invalid cron expression: {}", e),
                        };
                        println!(
                            "Scheduler: Registered '{}' ({}) -> {}",
                            task.archive_name, cron_str, next_run_info
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
            // After starting scheduled loops, detect recent missed occurrences and optionally run them now
            // (useful when tasks were added for times already passed and `run_on_boot_if_missed` is true)
            for (task, cron_str) in current_tasks.iter() {
                if !task.schedule.run_on_boot_if_missed {
                    continue;
                }

                // Only consider missed-run for tasks that are newly added compared to prev_names
                // If a task with the same name existed before, we treat this as an update and skip missed-run.
                let is_new = !prev_names.contains(&task.task_name);
                if !is_new {
                    // Skip missed-run for tasks that existed before reload (updates)
                    continue;
                }

                match Schedule::from_str(cron_str) {
                    Ok(schedule) => {
                        let now: chrono::DateTime<chrono::Utc> = chrono::Utc::now();
                        // Look for the next occurrence after a short-lookback window (e.g., 5 minutes)
                        let lookback = chrono::Duration::minutes(5);
                        let from: chrono::DateTime<chrono::Utc> = now - lookback;
                        let next: Option<chrono::DateTime<chrono::Utc>> = schedule.after(&from).next();
                        if let Some(next) = next {
                            if next <= now {
                                // If we already recorded a run at or after `next`, skip executing it again.
                                let already_ran = match task.last_run {
                                    Some(ts) => {
                                        // Convert SystemTime to chrono DateTime<Utc> for comparison
                                        let lr: chrono::DateTime<chrono::Utc> = chrono::DateTime::from(ts);
                                        lr >= next
                                    }
                                    None => false,
                                };

                                if already_ran {
                                    // Skip because task already ran at or after this scheduled occurrence
                                    println!("Scheduler: Detected missed run for '{}' at {} but last_run >= that, skipping", task.archive_name, next);
                                } else {
                                    // We consider this a missed occurrence; run it immediately in background
                                    let cron_task = BorgBackupTask {
                                        task_data: task.clone(),
                                        reporter: self.reporter.clone(),
                                    };
                                    let reporter_clone = self.reporter.clone();
                                    println!("Scheduler: Missed run detected for '{}' (scheduled at {}), executing now", task.archive_name, next);
                                    // Run in background
                                    tokio::spawn(async move {
                                        let _ = cron_task.execute(TaskContext {
                                            scheduled_time: chrono::Utc::now(),
                                            actual_time: chrono::Utc::now(),
                                            weight: 0,
                                            metadata: std::collections::HashMap::new(),
                                        }).await;
                                        if let Some(r) = &reporter_clone {
                                            r.on_scheduler_tick(0);
                                        }
                                    });
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Scheduler: cannot parse cron for missed-run check '{}': {}", cron_str, e);
                    }
                }
            }
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
                                    // capture previous keys before replacing tasks
                                    let old_tasks = self.tasks.lock().unwrap().clone();
                                    let mut old_names: HashSet<String> = HashSet::new();
                                    for (t, _cron) in old_tasks.iter() {
                                        old_names.insert(t.task_name.clone());
                                    }

                                    let new_tasks = reloader();
                                    println!("Scheduler: Reloaded {} tasks from external source", new_tasks.len());
                                    *self.tasks.lock().unwrap() = new_tasks;
                                    // update prev_names to the old names so next outer iteration can detect newly added tasks
                                    prev_names = old_names;
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
