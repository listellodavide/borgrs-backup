slint::include_modules!();

pub mod app_state;
pub mod bridge;
pub mod commands;
pub mod scheduler_bridge;
pub mod task_runner;

use app_state::BorgAppState as RustAppState;
use async_trait::async_trait;
use borg_core::scheduler::{
    BackupSchedule as CoreBackupSchedule, ScheduleType as CoreScheduleType,
    ScheduledTask as CoreScheduledTask, Scheduler, SchedulerReporter,
};
use slint::{ComponentHandle, Model};
use std::sync::{Arc, Mutex};

struct GuiSchedulerReporter {
    window_weak: slint::Weak<MainWindow>,
    state: Arc<Mutex<RustAppState>>,
}

impl std::fmt::Debug for GuiSchedulerReporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuiSchedulerReporter").finish()
    }
}

#[async_trait]
impl SchedulerReporter for GuiSchedulerReporter {
    fn on_task_start(&self, task: &CoreScheduledTask) {
        let _ = slint::invoke_from_event_loop({
            let window_weak = self.window_weak.clone();
            let repo_path = task.repo_path.clone();
            let task_name = task.archive_name.clone();
            move || {
                if let Some(window) = window_weak.upgrade() {
                    println!(
                        "Scheduled task '{}' started for repo: {}",
                        task_name, repo_path
                    );
                    window.global::<AppState>().set_is_processing(true);
                    window.global::<AppState>().set_progress(0.0);
                    let dash = window.global::<DashboardLogic>();
                    dash.set_is_scheduled_backup_running(true);
                    dash.set_scheduled_backup_repo_path(repo_path.clone().into());
                    dash.set_terminal_text(
                        format!(
                            "Scheduled task '{}' started for repo: {}",
                            task_name, repo_path
                        )
                        .into(),
                    );
                }
            }
        });
    }

    fn on_task_progress(&self, _task: &CoreScheduledTask, processed: u64, total: u64) {
        let _ = slint::invoke_from_event_loop({
            let window_weak = self.window_weak.clone();
            let progress = if total > 0 {
                processed as f32 / total as f32
            } else {
                0.0
            };
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
            let task_name = task.task_name.clone();
            let state_arc = self.state.clone();
            let task_id = {
                let s = state_arc.lock().unwrap();
                s.scheduled_tasks
                    .iter()
                    .find(|t| t.task_name == task_name)
                    .and_then(|t| t.id)
            };

            move || {
                if let Some(window) = window_weak.upgrade() {
                    println!(
                        "Scheduled task '{}' completed for {}: {}",
                        task_name, repo_path, summary
                    );
                    window.global::<AppState>().set_is_processing(false);
                    window.global::<AppState>().set_progress(1.0);
                    let dash = window.global::<DashboardLogic>();
                    dash.set_is_scheduled_backup_running(false);
                    dash.set_terminal_text(
                        format!(
                            "Scheduled task '{}' completed for {}: {}",
                            task_name, repo_path, summary
                        )
                        .into(),
                    );

                    // Update database execution stats
                    if let Some(id) = task_id {
                        let db = {
                            let s = state_arc.lock().unwrap();
                            s.db.clone()
                        };
                        if let Some(db) = db {
                            let timestamp = borg_core::archive::current_time_hh_mm_dd_mm_yyyy();
                            tokio::spawn(async move {
                                let _ = db.increment_task_execution(id, timestamp).await;
                            });
                        }
                    }
                }
            }
        });
    }

    fn on_task_error(&self, task: &CoreScheduledTask, error: String) {
        let _ = slint::invoke_from_event_loop({
            let window_weak = self.window_weak.clone();
            let repo_path = task.repo_path.clone();
            let task_name = task.archive_name.clone();
            move || {
                if let Some(window) = window_weak.upgrade() {
                    println!(
                        "Scheduled task '{}' failed for {}: {}",
                        task_name, repo_path, error
                    );
                    window.global::<AppState>().set_is_processing(false);
                    let dash = window.global::<DashboardLogic>();
                    dash.set_is_scheduled_backup_running(false);
                    if error.contains("Failed to acquire lock") {
                        dash.set_terminal_text(
                            format!(
                                "Task '{}' for {} is queued, waiting for lock.",
                                task_name, repo_path
                            )
                            .into(),
                        );
                    } else {
                        dash.set_terminal_text(
                            format!(
                                "Scheduled task '{}' failed for {}: {}",
                                task_name, repo_path, error
                            )
                            .into(),
                        );
                    }
                }
            }
        });
    }

    async fn get_password(
        &self,
        _repo_path: &str,
        repo_name: &str,
        retry: usize,
    ) -> Option<String> {
        let state_arc = self.state.clone();

        // On first try, check keyring (account borgrs_<name>)
        if retry == 0 {
            let state = state_arc.lock().unwrap();
            if let Ok(pwd) = state.get_password(repo_name) {
                return Some(pwd);
            }
        }

        // Otherwise (or if keyring fails), ask user
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut state = state_arc.lock().unwrap();
            state.auth_oneshot = Some(tx);
        }

        let _ = slint::invoke_from_event_loop({
            let window_weak = self.window_weak.clone();
            let repo_path = _repo_path.to_string();
            let repo_name = repo_name.to_string();
            let retry_count = retry;
            move || {
                if let Some(window) = window_weak.upgrade() {
                    let app = window.global::<AppState>();
                    app.set_pending_auth_repo_path(repo_path.into());
                    app.set_show_password_dialog(true);

                    let dash = window.global::<DashboardLogic>();
                    let msg = if retry_count > 0 {
                        format!(
                            "Incorrect password (attempt {}/3). Please enter password for '{}':",
                            retry_count, repo_name
                        )
                    } else {
                        format!("Password required for scheduled backup of '{}':", repo_name)
                    };
                    dash.set_terminal_text(msg.into());
                }
            }
        });

        // Wait for user input (oneshot will be completed in bridge.rs)
        match rx.await {
            Ok(pwd) => Some(pwd),
            Err(_) => None, // Cancelled or closed
        }
    }

    fn on_file_processed(&self, _task: &borg_core::scheduler::ScheduledTask, filename: &str) {
        let _ = slint::invoke_from_event_loop({
            let window_weak = self.window_weak.clone();
            let file = filename.to_string();
            move || {
                if let Some(window) = window_weak.upgrade() {
                    let dash = window.global::<DashboardLogic>();
                    let mut files: Vec<slint::SharedString> =
                        dash.get_processed_files().iter().cloned().collect();
                    files.insert(0, file.into());
                    if files.len() > 50 {
                        files.truncate(50);
                    }
                    dash.set_processed_files(slint::VecModel::from(files).into());
                }
            }
        });
    }

    fn on_scheduler_tick(&self, tasks_found: usize) {
        let _ = slint::invoke_from_event_loop({
            let window_weak = self.window_weak.clone();
            move || {
                if let Some(window) = window_weak.upgrade() {
                    let dash = window.global::<DashboardLogic>();
                    let timestamp = borg_core::archive::current_time_hh_mm_dd_mm_yyyy();
                    if tasks_found > 0 {
                        dash.set_status_text(
                            format!(
                                "Scheduled Task check OK, run {} tasks at {}",
                                tasks_found, timestamp
                            )
                            .into(),
                        );
                    } else {
                        dash.set_status_text(
                            format!("Scheduled Task check OK, None at {}", timestamp).into(),
                        );
                    }
                }
            }
        });
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // Initialize Database
    let mut db_path = std::env::current_exe()?;
    db_path.pop();
    db_path.push("borg-gui.db");
    let db = Arc::new(borg_core::db::Database::new(&db_path).await?);

    let main_window = MainWindow::new()?;
    let rust_app_state = Arc::new(Mutex::new(RustAppState::new()));

    // Load state from database
    {
        let mut state = rust_app_state.lock().unwrap();
        state.set_db(db.clone());
        state.load_all().await?;
    }

    // Set initial data for DashboardLogic
    {
        let dashboard = main_window.global::<DashboardLogic>();

        // Load bookmarks from Rust app state
        let bookmarks = {
            let state = rust_app_state.lock().unwrap();
            state.bookmarks.clone()
        };

        let repos: Vec<RepoItem> = bookmarks
            .into_iter()
            .map(|b| RepoItem {
                name: b.name.into(),
                path: b.path.into(),
                repo_type: b.repo_type.into(),
            })
            .collect();

        let repos_model = std::rc::Rc::new(slint::VecModel::from(repos));
        dashboard.set_repositories(repos_model.into());

        // Load archives for the first repository if it exists
        if let Some(first_repo) = dashboard.get_repositories().iter().next() {
            let repo_path = first_repo.path.to_string();
            let password = {
                let state = rust_app_state.lock().unwrap();
                state.session_passwords.get(&repo_path).cloned()
            };

            let window_weak = main_window.as_weak();
            tokio::spawn(async move {
                match async {
                    let storage = borg_core::storage::StorageConfig::Local {
                        path: std::path::PathBuf::from(&repo_path),
                    };
                    let op = borg_core::storage::build_operator(storage)
                        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                    let repo = borg_core::repository::Repository::open(
                        op,
                        repo_path.clone(),
                        password.as_deref(),
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                    let manifest = repo
                        .load_manifest()
                        .await
                        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                    Ok::<_, anyhow::Error>(manifest.archives)
                }
                .await
                {
                    Ok(archive_list) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = window_weak.upgrade() {
                                let archive_entries: Vec<ArchiveEntry> = archive_list
                                    .into_iter()
                                    .map(|archive| ArchiveEntry {
                                        name: archive.name.clone().into(),
                                        date: archive.time.to_string().into(),
                                        size: "".into(),
                                        hostname: archive.hostname.clone().into(),
                                        comment: archive.comment.unwrap_or_default().into(),
                                        tags: archive.tags.unwrap_or_default().join(", ").into(),
                                    })
                                    .collect();
                                let archives_model =
                                    std::rc::Rc::new(slint::VecModel::from(archive_entries));
                                w.global::<DashboardLogic>()
                                    .set_archives(archives_model.into());
                            }
                        });
                    }
                    Err(e) => {
                        let error_msg = format!("Failed to load archives: {}", e);
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = window_weak.upgrade() {
                                w.global::<DashboardLogic>()
                                    .set_terminal_text(error_msg.into());
                            }
                        });
                    }
                }
            });
        } else {
            let archives_model = std::rc::Rc::new(slint::VecModel::from(vec![]));
            dashboard.set_archives(archives_model.into());
        }

        dashboard
            .set_terminal_text("Welcome to Borg Backup Disaster Recovery client is ready!".into());
        dashboard.set_status_text(
            format!(
                "All systems OK, {}",
                borg_core::archive::current_time_hh_mm_dd_mm_yyyy()
            )
            .into(),
        );
    }

    // Start the scheduler
    let scheduled_tasks = {
        let state = rust_app_state.lock().unwrap();
        state
            .scheduled_tasks
            .iter()
            // Only include active tasks and exclude Manual tasks (those are triggered manually)
            .filter(|t| t.active && t.schedule_type != "Manual")
            .map(|task| {
                let core_task = CoreScheduledTask {
                    task_name: task.task_name.clone(),
                    repo_name: task.repo_name.clone(),
                    repo_path: task.repo_path.clone(),
                    archive_name: task.archive_name.clone(),
                    paths_to_backup: task.paths_to_backup.clone(),
                    compression: task.compression.clone(),
                    comment: task.comment.clone(),
                    tags: task
                        .tags
                        .as_ref()
                        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect()),
                    schedule: CoreBackupSchedule {
                        schedule_type: match task.schedule_type.as_str() {
                            "Daily" => CoreScheduleType::Daily,
                            "Weekly" => CoreScheduleType::Weekly,
                            "Monthly" => CoreScheduleType::Monthly,
                            _ => CoreScheduleType::Manual,
                        },
                        weekday: task.weekday as u32,
                        day_of_month: task.day_of_month as u32,
                        hour: task.hour as u32,
                        minute: task.minute as u32,
                        run_on_boot_if_missed: task.run_on_boot_if_missed,
                    },
                    last_run: None,
                    execution_count: task.execution_count as u32,
                };
                let cron_str = core_task.schedule.to_cron_string();
                (core_task, cron_str)
            })
            .collect()
    };

    // Start the scheduler
    let scheduler = Scheduler::new(scheduled_tasks).with_reporter(Arc::new(GuiSchedulerReporter {
        window_weak: main_window.as_weak(),
        state: rust_app_state.clone(),
    }));

    // Set up task reloader callback for dynamic reloading from app_state
    let app_state_for_reload = rust_app_state.clone();
    let _ = Scheduler::set_task_reloader(move || {
        let state = app_state_for_reload.lock().unwrap();
        state
            .scheduled_tasks
            .iter()
            // Only include active tasks and exclude Manual tasks
            .filter(|t| t.active && t.schedule_type != "Manual")
            .map(|task| {
                let core_task = CoreScheduledTask {
                    task_name: task.task_name.clone(),
                    repo_name: task.repo_name.clone(),
                    repo_path: task.repo_path.clone(),
                    archive_name: task.archive_name.clone(),
                    paths_to_backup: task.paths_to_backup.clone(),
                    compression: task.compression.clone(),
                    comment: task.comment.clone(),
                    tags: task
                        .tags
                        .as_ref()
                        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect()),
                    schedule: CoreBackupSchedule {
                        schedule_type: match task.schedule_type.as_str() {
                            "Daily" => CoreScheduleType::Daily,
                            "Weekly" => CoreScheduleType::Weekly,
                            "Monthly" => CoreScheduleType::Monthly,
                            _ => CoreScheduleType::Manual,
                        },
                        weekday: task.weekday as u32,
                        day_of_month: task.day_of_month as u32,
                        hour: task.hour as u32,
                        minute: task.minute as u32,
                        run_on_boot_if_missed: task.run_on_boot_if_missed,
                    },
                    last_run: None,
                    execution_count: task.execution_count as u32,
                };
                let cron_str = core_task.schedule.to_cron_string();
                (core_task, cron_str)
            })
            .collect()
    });

    tokio::spawn(async move {
        scheduler.run().await;
    });

    bridge::init_bridge(&main_window, rust_app_state.clone());
    scheduler_bridge::init_scheduler_bridge(&main_window, rust_app_state.clone());

    main_window.run()?;
    Ok(())
}
