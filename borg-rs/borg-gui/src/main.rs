slint::include_modules!();

pub mod app_state;
pub mod commands;
pub mod bridge;
pub mod scheduler_bridge;
pub mod task_runner;

use std::sync::{Arc, Mutex};
use app_state::BorgAppState as RustAppState;
use slint::ComponentHandle;
use borg_core::scheduler::{Scheduler, ScheduledTask as CoreScheduledTask, BackupSchedule as CoreBackupSchedule, ScheduleType as CoreScheduleType, SchedulerReporter};

struct GuiSchedulerReporter {
    window_weak: slint::Weak<MainWindow>,
    state: Arc<Mutex<RustAppState>>,
}

impl SchedulerReporter for GuiSchedulerReporter {
    fn on_task_start(&self, task: &CoreScheduledTask) {
        let _ = slint::invoke_from_event_loop({
            let window_weak = self.window_weak.clone();
            let repo_path = task.repo_path.clone();
            move || {
                if let Some(window) = window_weak.upgrade() {
                    window.global::<AppState>().set_is_processing(true);
                    window.global::<AppState>().set_progress(0.0);
                    let dash = window.global::<DashboardLogic>();
                    dash.set_is_scheduled_backup_running(true);
                    dash.set_scheduled_backup_repo_path(repo_path.clone().into());
                    dash.set_terminal_text(format!("Scheduled task started for repo: {}", repo_path).into());
                }
            }
        });
    }

    fn on_task_progress(&self, _task: &CoreScheduledTask, processed: u64, total: u64) {
        let _ = slint::invoke_from_event_loop({
            let window_weak = self.window_weak.clone();
            let progress = if total > 0 { processed as f32 / total as f32 } else { 0.0 };
            move || {
                if let Some(window) = window_weak.upgrade() {
                    window.global::<AppState>().set_progress(progress);
                }
            }
        });
    }

    fn on_task_complete(&self, task: &CoreScheduledTask, summary: String) {
        let _ = slint::invoke_from_event_loop({
            let window_weak = self.window_weak.clone();
            let repo_path = task.repo_path.clone();
            move || {
                if let Some(window) = window_weak.upgrade() {
                    window.global::<AppState>().set_is_processing(false);
                    window.global::<AppState>().set_progress(1.0);
                    let dash = window.global::<DashboardLogic>();
                    dash.set_is_scheduled_backup_running(false);
                    dash.set_terminal_text(format!("Scheduled task completed for {}: {}", repo_path, summary).into());
                }
            }
        });
    }

    fn on_task_error(&self, task: &CoreScheduledTask, error: String) {
        let _ = slint::invoke_from_event_loop({
            let window_weak = self.window_weak.clone();
            let repo_path = task.repo_path.clone();
            move || {
                if let Some(window) = window_weak.upgrade() {
                    window.global::<AppState>().set_is_processing(false);
                    let dash = window.global::<DashboardLogic>();
                    dash.set_is_scheduled_backup_running(false);
                    if error.contains("Failed to acquire lock") {
                        dash.set_terminal_text(format!("Task for {} is queued, waiting for lock.", repo_path).into());
                    } else {
                        dash.set_terminal_text(format!("Scheduled task failed for {}: {}", repo_path, error).into());
                    }
                }
            }
        });
    }

    fn get_password(&self, repo_path: &str) -> Option<String> {
        let state = self.state.lock().unwrap();
        // Check session passwords first
        if let Some(pwd) = state.session_passwords.get(repo_path) {
            return Some(pwd.clone());
        }
        // Then check if it's a bookmark name and we have password for its path
        if let Some(bm) = state.bookmarks.iter().find(|b| b.name == repo_path) {
            if let Some(pwd) = state.session_passwords.get(&bm.path) {
                return Some(pwd.clone());
            }
            // Check keyring for bookmark path
            if let Ok(pwd) = state.get_password(&bm.path) {
                return Some(pwd);
            }
        }
        // Check keyring for repo_path directly
        if let Ok(pwd) = state.get_password(repo_path) {
            return Some(pwd);
        }
        None
    }

    fn on_scheduler_tick(&self, tasks_found: usize) {
        let _ = slint::invoke_from_event_loop({
            let window_weak = self.window_weak.clone();
            move || {
                if let Some(window) = window_weak.upgrade() {
                    let dash = window.global::<DashboardLogic>();
                    let timestamp = borg_core::archive::current_time_hh_mm_dd_mm_yyyy();
                    if tasks_found > 0 {
                        dash.set_status_text(format!("Scheduled Task check OK, run {} tasks at {}", tasks_found, timestamp).into());
                    } else {
                        dash.set_status_text(format!("Scheduled Task check OK, None at {}", timestamp).into());
                    }
                }
            }
        });
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let main_window = MainWindow::new()?;
    let rust_app_state = Arc::new(Mutex::new(RustAppState::new()));

    // Set initial data for DashboardLogic
    {
        let dashboard = main_window.global::<DashboardLogic>();
        
        // Load bookmarks from Rust app state
        let bookmarks = {
            let state = rust_app_state.lock().unwrap();
            state.bookmarks.clone()
        };

        let repos: Vec<RepoItem> = bookmarks.into_iter().map(|b| RepoItem {
            name: b.name.into(),
            path: b.path.into(),
            repo_type: b.repo_type.into(),
        }).collect();

        let archives = vec![];
        
        let repos_model = std::rc::Rc::new(slint::VecModel::from(repos));
        dashboard.set_repositories(repos_model.into());
        
        let archives_model = std::rc::Rc::new(slint::VecModel::from(archives));
        dashboard.set_archives(archives_model.into());
        
        dashboard.set_terminal_text("Welcome to Borg Backup Disaster Recovery client is ready!".into());
        dashboard.set_status_text(format!("All systems OK, {}", borg_core::archive::current_time_hh_mm_dd_mm_yyyy()).into());
    }

    // Start the scheduler
    let scheduled_tasks = {
        let state = rust_app_state.lock().unwrap();
        state.scheduled_tasks.iter().map(|task| {
            let archive_bookmark = state.get_archive_bookmark_for_repo(&task.repo_name);
            CoreScheduledTask {
                repo_path: task.repo_name.clone(),
                archive_name: task.archive_name.clone(),
                paths_to_backup: archive_bookmark.as_ref().map_or(vec![], |ab| ab.paths.clone()),
                compression: archive_bookmark.as_ref().map_or("zstd,3".to_string(), |ab| ab.compression.clone()),
                comment: archive_bookmark.as_ref().and_then(|ab| ab.comment.clone()),
                tags: archive_bookmark.as_ref().and_then(|ab| ab.tags.as_ref().map(|t| t.split(',').map(|s| s.trim().to_string()).collect())),
                schedule: CoreBackupSchedule {
                    schedule_type: match task.schedule.schedule_type {
                        app_state::ScheduleType::Daily => CoreScheduleType::Daily,
                        app_state::ScheduleType::Weekly => CoreScheduleType::Weekly,
                        app_state::ScheduleType::Monthly => CoreScheduleType::Monthly,
                        app_state::ScheduleType::Manual => CoreScheduleType::Manual,
                    },
                    weekday: task.schedule.weekday as u32,
                    day_of_month: task.schedule.day_of_month as u32,
                    hour: task.schedule.hour as u32,
                    minute: task.schedule.minute as u32,
                    run_on_boot_if_missed: task.schedule.run_on_boot_if_missed,
                },
                last_run: None,
            }
        }).collect()
    };
    let scheduler = Scheduler::new(scheduled_tasks)
        .with_reporter(Arc::new(GuiSchedulerReporter {
            window_weak: main_window.as_weak(),
            state: rust_app_state.clone(),
        }));
    tokio::spawn(async move {
        scheduler.run().await;
    });

    bridge::init_bridge(&main_window, rust_app_state.clone());
    scheduler_bridge::init_scheduler_bridge(&main_window, rust_app_state.clone());

    main_window.run()?;
    Ok(())
}
