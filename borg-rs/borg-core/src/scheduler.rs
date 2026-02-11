use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use chrono::{DateTime, Datelike, Local, Timelike};
use tokio::time::sleep;

#[derive(Debug, Clone, PartialEq)]
pub enum ScheduleType {
    Daily,
    Weekly,
    Monthly,
    Manual,
}

#[derive(Debug, Clone)]
pub struct BackupSchedule {
    pub schedule_type: ScheduleType,
    pub weekday: u32,      // 0 = Mon … 6 = Sun
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
    pub last_run: Option<SystemTime>,
    pub execution_count: u32,
}

pub trait SchedulerReporter: Send + Sync {
    fn on_task_start(&self, task: &ScheduledTask);
    fn on_task_progress(&self, task: &ScheduledTask, processed: u64, total: u64);
    fn on_task_complete(&self, task: &ScheduledTask, summary: String);
    fn on_task_error(&self, task: &ScheduledTask, error: String);
    fn get_password(&self, repo_path: &str) -> Option<String>;
    fn on_scheduler_tick(&self, tasks_found: usize);
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
        println!("Scheduler started");
        loop {
            let now = Local::now();
            let mut tasks_to_run = Vec::new();

            // Scope for lock
            {
                let mut tasks = self.tasks.lock().unwrap();
                for task in tasks.iter_mut() {
                    if self.should_run(task, &now) {
                        println!("Task due: {}", task.archive_name);
                        tasks_to_run.push(task.clone());
                        task.last_run = Some(SystemTime::now());
                        task.execution_count += 1;
                    }
                }
            }

            if let Some(reporter) = &self.reporter {
                reporter.on_scheduler_tick(tasks_to_run.len());
            }

            for task in tasks_to_run {
                self.execute_task(task).await;
            }

            sleep(Duration::from_secs(60)).await;
        }
    }

    fn should_run(&self, task: &ScheduledTask, now: &DateTime<Local>) -> bool {
        if task.schedule.schedule_type == ScheduleType::Manual {
            return false;
        }

        // Check if time matches (minute precision)
        if now.hour() != task.schedule.hour || now.minute() != task.schedule.minute {
            return false;
        }

        match task.schedule.schedule_type {
            ScheduleType::Daily => true,
            ScheduleType::Weekly => {
                // chrono weekday: Mon=0, Sun=6
                now.weekday().num_days_from_monday() == task.schedule.weekday
            }
            ScheduleType::Monthly => {
                now.day() == task.schedule.day_of_month
            }
            ScheduleType::Manual => false,
        }
    }

    async fn execute_task(&self, task: ScheduledTask) {
        if let Some(reporter) = &self.reporter {
            reporter.on_task_start(&task);
        }

        let password = if let Some(reporter) = &self.reporter {
            reporter.get_password(&task.repo_path)
        } else {
            None
        };

        // Construct archive name with timestamp
        let timestamp = crate::archive::current_time_for_archive_name();
        let archive_name = format!("{}-{}", task.archive_name, timestamp);

        println!("Executing backup for {}", archive_name);

        // Perform the actual backup
        let storage = crate::storage::StorageConfig::Local {
            path: std::path::PathBuf::from(&task.repo_path)
        };

        match crate::storage::build_operator(storage) {
            Ok(op) => {
                match crate::repository::Repository::open(op, task.repo_path.clone(), password.as_deref()).await {
                    Ok(mut repo) => {
                        let paths: Vec<std::path::PathBuf> = task.paths_to_backup.iter().map(std::path::PathBuf::from).collect();

                        // Create progress callback
                        let reporter_clone = self.reporter.clone();
                        let task_clone = task.clone();
                        let progress_callback = move |processed: u64, total: u64| {
                            if let Some(r) = &reporter_clone {
                                r.on_task_progress(&task_clone, processed, total);
                            }
                        };

                        match repo.create_archive(
                            &archive_name,
                            &paths,
                            &task.compression,
                            task.comment.as_deref(),
                            task.tags.as_deref(),
                            progress_callback
                        ).await {
                            Ok(summary) => {
                                if let Some(reporter) = &self.reporter {
                                    reporter.on_task_complete(&task, summary);
                                }
                            }
                            Err(e) => {
                                if let Some(reporter) = &self.reporter {
                                    reporter.on_task_error(&task, e.to_string());
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if let Some(reporter) = &self.reporter {
                            reporter.on_task_error(&task, format!("Failed to open repository: {}", e));
                        }
                    }
                }
            }
            Err(e) => {
                if let Some(reporter) = &self.reporter {
                    reporter.on_task_error(&task, format!("Failed to build storage operator: {}", e));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;
    use crate::storage::{StorageConfig, build_operator};
    use crate::repository::Repository;
    use std::fs;

    struct TestReporter {
        started: Arc<Mutex<bool>>,
        completed: Arc<Mutex<bool>>,
    }

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
        fn get_password(&self, _repo_path: &str) -> Option<String> {
            Some("hello".to_string())
        }
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
        let op = build_operator(StorageConfig::Local { path: repo_path.clone() }).unwrap();
        let _ = Repository::init(op, repo_path.to_string_lossy().to_string(), Some("hello"), None).await.unwrap();

        let now = Local::now() + chrono::Duration::minutes(2);
        let task = ScheduledTask {
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
                hour: now.hour(),
                minute: now.minute(),
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

        let scheduler = Scheduler::new(vec![task]).with_reporter(reporter);

        // We need to mock time or wait. Since we can't easily mock time in this setup without refactoring,
        // we will manually trigger the check logic with a modified "now" in a separate testable function,
        // or just verify the logic.
        // However, for this integration test, we can just call execute_task directly to verify it works end-to-end.

        let tasks = scheduler.tasks.lock().unwrap();
        let task_to_run = tasks[0].clone();
        drop(tasks);

        scheduler.execute_task(task_to_run).await;

        assert!(*started.lock().unwrap());
        assert!(*completed.lock().unwrap());
    }
}
