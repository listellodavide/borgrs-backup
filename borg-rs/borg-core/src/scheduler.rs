use crate::archive::{ArchiveCreator, BackupProgress};
use crate::repository::Repository;
use chrono::{Datelike, Local, Timelike};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};
use std::path::Path;

pub trait SchedulerReporter: Send + Sync {
    fn on_task_start(&self, task: &ScheduledTask);
    fn on_task_progress(&self, task: &ScheduledTask, processed: u64, total: u64);
    fn on_task_complete(&self, task: &ScheduledTask, summary: String);
    fn on_task_error(&self, task: &ScheduledTask, error: String);
    fn get_password(&self, repo_path: &str) -> Option<String>;
}

struct SchedulerBackupProgress {
    reporter: Arc<dyn SchedulerReporter>,
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

#[derive(Debug, Clone)]
pub enum ScheduleType {
    Daily,
    Weekly,
    Manual,
}

#[derive(Debug, Clone)]
pub struct BackupSchedule {
    pub schedule_type: ScheduleType,
    pub weekday: u32, // 0 = Mon … 6 = Sun
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
}

pub struct Scheduler {
    tasks: Arc<Mutex<Vec<ScheduledTask>>>,
    reporter: Option<Arc<dyn SchedulerReporter>>,
}

impl Scheduler {
    pub fn new(tasks: Vec<ScheduledTask>) -> Self {
        Self {
            tasks: Arc::new(Mutex::new(tasks)),
            reporter: None,
        }
    }

    pub fn with_reporter(mut self, reporter: Arc<dyn SchedulerReporter>) -> Self {
        self.reporter = Some(reporter);
        self
    }

    pub async fn run(&self) {
        loop {
            let now = Local::now();
            let mut tasks = self.tasks.lock().await;

            for task in tasks.iter_mut() {
                if self.should_run(task, &now) {
                    self.run_task(task.clone()).await;
                    task.last_run = Some(now);
                }
            }

            drop(tasks);
            sleep(Duration::from_secs(60)).await; // Check every minute
        }
    }

    fn should_run(&self, task: &ScheduledTask, now: &chrono::DateTime<Local>) -> bool {
        if let Some(last_run) = task.last_run {
            if now.signed_duration_since(last_run).num_minutes() < 1 {
                return false; // Avoid running the same task multiple times in a minute
            }
        }

        let schedule = &task.schedule;
        if now.hour() != schedule.hour || now.minute() != schedule.minute {
            return false;
        }

        match schedule.schedule_type {
            ScheduleType::Daily => true,
            ScheduleType::Weekly => now.weekday().num_days_from_monday() == schedule.weekday,
            ScheduleType::Manual => false,
        }
    }

    async fn run_task(&self, mut task: ScheduledTask) {
        if let Some(reporter) = &self.reporter {
            reporter.on_task_start(&task);
        } else {
            println!("Running scheduled task: backing up to {}", task.repo_path);
        }

        // Generate archive name based on schedule type if it's following the placeholder pattern or empty
        let now = Local::now();
        let timestamp = now.format("%Y-%m-%d-%H-%M").to_string();
        let prefix = match task.schedule.schedule_type {
            ScheduleType::Daily => "daily",
            ScheduleType::Weekly => "weekly",
            ScheduleType::Manual => "manual",
        };
        
        // Use the requested format: prefix-yyyy-mm-dd-hh-MM
        // We override the name for scheduled tasks to ensure it follows the convention
        task.archive_name = format!("{}-{}", prefix, timestamp);

        let password = self.reporter.as_ref().and_then(|r| r.get_password(&task.repo_path));

        let mut repo = match Repository::open(
            crate::storage::build_operator(crate::storage::StorageConfig::Local { path: task.repo_path.clone().into() }).unwrap(),
            task.repo_path.clone(),
            password.as_deref(),
        )
        .await
        {
            Ok(repo) => repo,
            Err(e) => {
                let err_msg = format!("Failed to open repository: {}", e);
                if let Some(reporter) = &self.reporter {
                    reporter.on_task_error(&task, err_msg);
                } else {
                    eprintln!("{}", err_msg);
                }
                return;
            }
        };

        let mut creator = ArchiveCreator::new(&mut repo);
        if let Some(reporter) = &self.reporter {
            creator = creator.with_progress(Box::new(SchedulerBackupProgress {
                reporter: reporter.clone(),
                task: task.clone(),
            }));
        }

        let result = creator
            .create(
                &task.archive_name,
                &task.paths_to_backup.iter().map(|p| p.into()).collect::<Vec<_>>(),
                task.comment.clone(),
                task.tags.clone(),
            )
            .await;

        match result {
            Ok(summary) => {
                let summary_str = format!("{:?}", summary);
                if let Some(reporter) = &self.reporter {
                    reporter.on_task_complete(&task, summary_str);
                } else {
                    println!("Scheduled backup completed: {}", summary_str);
                }
            }
            Err(e) => {
                let err_msg = format!("Scheduled backup failed: {}", e);
                if let Some(reporter) = &self.reporter {
                    reporter.on_task_error(&task, err_msg);
                } else {
                    eprintln!("{}", err_msg);
                }
            }
        }
    }
}
