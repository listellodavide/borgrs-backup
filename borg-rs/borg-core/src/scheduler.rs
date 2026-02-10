use crate::archive::{ArchiveCreator, BackupProgress};
use crate::repository::Repository;
use crate::lock::RepositoryLock;
use chrono::{Datelike, Local, Timelike, Duration}; // Removed Weekday
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::sleep;
use std::path::Path;

pub trait SchedulerReporter: Send + Sync {
    fn on_task_start(&self, task: &ScheduledTask);
    fn on_task_progress(&self, task: &ScheduledTask, processed: u64, total: u64);
    fn on_task_complete(&self, task: &ScheduledTask, summary: String);
    fn on_task_error(&self, task: &ScheduledTask, error: String);
    fn get_password(&self, repo_path: &str) -> Option<String>;
    fn on_scheduler_tick(&self, tasks_found: usize);
}

struct SchedulerBackupProgress {
    reporter: Arc<dyn SchedulerReporter + Send + Sync>, // Changed to include Send + Sync
    task: ScheduledTask,
}

impl BackupProgress for SchedulerBackupProgress {
    fn on_file_start(&self, _path: &Path) {}
    fn on_file_complete(&self, _path: &Path, _size: u64, _chunks: usize) {}
    fn on_file_skipped(&self, _path: &Path, _reason: &str) {}
    fn on_progress(&self, processed: u64, total: u64) {
        self.reporter.on_task_progress(&self.task, processed, total);
    }
    fn on_error(&self, _path: &Path, error: &str) {
        self.reporter.on_task_error(&self.task, error.to_string());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleType {
    Daily,
    Weekly,
    Monthly,
    Manual,
}

#[derive(Debug, Clone)]
pub struct BackupSchedule {
    pub schedule_type: ScheduleType,
    pub weekday: u32, // 0 = Mon … 6 = Sun
    pub day_of_month: u32, // 1-31 for monthly
    pub hour: u32,
    pub minute: u32,
    pub run_on_boot_if_missed: bool,
}

#[derive(Debug, Clone)]
pub struct ScheduledTask {
    pub repo_path: String,
    pub archive_name: String,
    pub paths_to_backup: Vec<String>,
    pub compression: String,
    pub comment: Option<String>,
    pub tags: Option<Vec<String>>,
    pub schedule: BackupSchedule,
    pub last_run: Option<chrono::DateTime<Local>>,
    pub execution_count: u32,
}

pub struct Scheduler {
    tasks: Arc<Mutex<Vec<ScheduledTask>>>,
    reporter: Option<Arc<dyn SchedulerReporter + Send + Sync>>, // Changed to include Send + Sync
    running_tasks: Arc<Mutex<HashSet<String>>>,
}

impl Scheduler {
    pub fn new(tasks: Vec<ScheduledTask>) -> Self {
        Self {
            tasks: Arc::new(Mutex::new(tasks)),
            reporter: None,
            running_tasks: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn with_reporter(mut self, reporter: Arc<dyn SchedulerReporter + Send + Sync>) -> Self { // Changed to include Send + Sync
        self.reporter = Some(reporter);
        self
    }

    pub async fn run(&self) {
        loop {
            let now = Local::now();
            let mut tasks_to_run = Vec::new();

            {
                let mut tasks = self.tasks.lock().await;
                let mut running_tasks = self.running_tasks.lock().await;

                for task in tasks.iter_mut() {
                    if task.schedule.schedule_type == ScheduleType::Manual {
                        continue;
                    }

                    if running_tasks.contains(&task.archive_name) {
                        continue; // Task is already running or queued
                    }

                    if self.should_run(task, &now) {
                        tasks_to_run.push(task.clone());
                        running_tasks.insert(task.archive_name.clone()); // Mark as running
                    }
                }
            }

            if let Some(reporter) = &self.reporter {
                reporter.on_scheduler_tick(tasks_to_run.len());
            }

            for mut task in tasks_to_run {
                let tasks_arc = self.tasks.clone();
                let running_tasks_arc = self.running_tasks.clone();
                let reporter_arc = self.reporter.clone(); // Clone the Arc for the spawned task

                tokio::spawn(async move {
                    Self::run_task(&mut task, reporter_arc).await; // Pass the Arc directly

                    // Update the main task list with the new last_run and execution_count
                    let mut tasks = tasks_arc.lock().await;
                    if let Some(t) = tasks.iter_mut().find(|t| t.archive_name == task.archive_name) {
                        t.last_run = Some(Local::now());
                        t.execution_count = task.execution_count;
                    }

                    // Remove from running set
                    running_tasks_arc.lock().await.remove(&task.archive_name);
                });
            }

            sleep(tokio::time::Duration::from_secs(30)).await;
        }
    }

    fn should_run(&self, task: &ScheduledTask, now: &chrono::DateTime<Local>) -> bool {
        let last_run = match task.last_run {
            Some(lr) => lr,
            None => {
                // If it has never run and run_on_boot is true, it's due.
                // Otherwise, treat its "last run" as now to schedule it for the future.
                return if task.schedule.run_on_boot_if_missed { true } else { false };
            }
        };

        let schedule = &task.schedule;
        let scheduled_time = now.with_hour(schedule.hour).unwrap().with_minute(schedule.minute).unwrap().with_second(0).unwrap();

        let next_run = match schedule.schedule_type {
            ScheduleType::Daily => {
                let next = last_run.date_naive().and_time(scheduled_time.time()) + Duration::days(1);
                next
            },
            ScheduleType::Weekly => {
                let days_to_add = (schedule.weekday as i64 - last_run.weekday().num_days_from_monday() as i64 + 7) % 7;
                let next_date = last_run.date_naive() + Duration::days(if days_to_add == 0 { 7 } else { days_to_add });
                next_date.and_time(scheduled_time.time())
            },
            ScheduleType::Monthly => {
                let mut next_month = last_run.month() + 1;
                let mut next_year = last_run.year();
                if next_month > 12 {
                    next_month = 1;
                    next_year += 1;
                }
                let last_day_of_next_month = chrono::NaiveDate::from_ymd_opt(next_year, next_month + 1, 1).unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(next_year + 1, 1, 1).unwrap()).pred_opt().unwrap().day();
                let day = std::cmp::min(schedule.day_of_month, last_day_of_next_month);
                chrono::NaiveDate::from_ymd_opt(next_year, next_month, day).unwrap().and_time(scheduled_time.time())
            },
            ScheduleType::Manual => return false,
        };

        *now >= next_run.and_local_timezone(Local).unwrap()
    }

    async fn run_task(task: &mut ScheduledTask, reporter: Option<Arc<dyn SchedulerReporter + Send + Sync>>) { // Changed reporter type
        if let Some(r) = &reporter {
            r.on_task_start(task);
        }

        let now = Local::now();
        let timestamp = now.format("%Y-%m-%d-%H-%M").to_string();
        let prefix = match task.schedule.schedule_type {
            ScheduleType::Daily => "daily",
            ScheduleType::Weekly => "weekly",
            ScheduleType::Monthly => "monthly",
            ScheduleType::Manual => "manual",
        };
        task.archive_name = format!("{}-{}", prefix, timestamp);

        let password = reporter.as_ref().and_then(|r| r.get_password(&task.repo_path));

        let repo_path_buf = std::path::PathBuf::from(&task.repo_path);
        let mut lock = RepositoryLock::new(&repo_path_buf);
        if let Err(e) = lock.acquire("scheduled_task", &task.archive_name) {
            if let Some(r) = &reporter {
                r.on_task_error(task, format!("Failed to acquire lock: {}", e));
            }
            return;
        }

        let repo_result = Repository::open(
            crate::storage::build_operator(crate::storage::StorageConfig::Local { path: task.repo_path.clone().into() }).unwrap(),
            task.repo_path.clone(),
            password.as_deref(),
        ).await;

        let mut repo = match repo_result {
            Ok(r) => r,
            Err(e) => {
                if let Some(r) = &reporter {
                    r.on_task_error(task, format!("Failed to open repository: {}", e));
                }
                return;
            }
        };

        let mut creator = ArchiveCreator::new(&mut repo);
        if let Some(r) = &reporter {
            let progress_reporter = SchedulerBackupProgress {
                reporter: r.clone(), // Clone the Arc
                task: task.clone(),
            };
            creator = creator.with_progress(Box::new(progress_reporter));
        }

        let result = creator.create(
            &task.archive_name,
            &task.paths_to_backup.iter().map(|p| p.into()).collect::<Vec<_>>(),
            task.comment.clone(),
            task.tags.clone(),
        ).await;

        match result {
            Ok(summary) => {
                task.execution_count += 1;
                if let Some(r) = &reporter {
                    r.on_task_complete(task, format!("{:?}", summary));
                }
            }
            Err(e) => {
                if let Some(r) = &reporter {
                    r.on_task_error(task, format!("Scheduled backup failed: {}", e));
                }
            }
        }
    }
}
