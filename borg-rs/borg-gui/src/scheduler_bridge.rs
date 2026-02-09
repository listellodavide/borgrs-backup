use std::sync::{Arc, Mutex};
use slint::{ComponentHandle, Model, VecModel};
use crate::app_state::{BorgAppState as RustAppState, ScheduledTask, BackupSchedule, ScheduleType};
use super::{MainWindow, SchedulerLogic, DashboardLogic};

// Generate task name from components
fn generate_task_name(repo_name: &str, archive_name: &str, schedule_type: i32, hour: i32, minute: i32) -> String {
    let freq = match schedule_type {
        0 => "daily",
        1 => "weekly",
        2 => "monthly",
        3 => "manual",
        _ => "manual",
    };
    format!("{}_{}_{}_{:02}_{:02}", repo_name, archive_name, freq, hour, minute)
        .to_lowercase()
        .replace(" ", "_")
}

pub fn init_scheduler_bridge(window: &MainWindow, state: Arc<Mutex<RustAppState>>) {
    let scheduler_logic = window.global::<SchedulerLogic>();
    let window_weak = window.as_weak();

    // Load initial repositories into scheduler
    let repos = {
        let s = state.lock().unwrap();
        s.bookmarks.clone()
    };
    let slint_repos: Vec<super::RepoItem> = repos.iter().map(|r| {
        super::RepoItem {
            name: r.name.clone().into(),
            path: r.path.clone().into(),
            repo_type: r.repo_type.clone().into(),
        }
    }).collect();

    let repo_names: Vec<slint::SharedString> = repos.iter()
        .map(|r| slint::SharedString::from(r.name.as_str()))
        .collect();

    scheduler_logic.set_available_repositories(std::rc::Rc::new(VecModel::from(slint_repos.clone())).into());
    scheduler_logic.set_available_repo_names(std::rc::Rc::new(VecModel::from(repo_names)).into());

    // Load initial tasks
    let tasks = {
        let s = state.lock().unwrap();
        s.scheduled_tasks.clone()
    };
    let slint_tasks: Vec<super::ScheduledTask> = tasks.into_iter().map(|t| {
        super::ScheduledTask {
            task_name: t.task_name.into(),
            repo_name: t.repo_name.into(),
            archive_name: t.archive_name.into(),
            schedule: super::BackupSchedule {
                schedule_type: match t.schedule.schedule_type {
                    ScheduleType::Daily => super::ScheduleType::Daily,
                    ScheduleType::Weekly => super::ScheduleType::Weekly,
                    ScheduleType::Monthly => super::ScheduleType::Monthly,
                    ScheduleType::Manual => super::ScheduleType::Manual,
                },
                weekday: t.schedule.weekday,
                day_of_month: t.schedule.day_of_month,
                hour: t.schedule.hour,
                minute: t.schedule.minute,
                run_on_boot_if_missed: t.schedule.run_on_boot_if_missed,
            }
        }
    }).collect();
    scheduler_logic.set_tasks(std::rc::Rc::new(VecModel::from(slint_tasks)).into());

    // Handle repository selection change
    scheduler_logic.on_on_repo_selected({
        let state_clone = state.clone();
        let window_weak = window_weak.clone();
        move |repo_index| {
            if let Some(window) = window_weak.upgrade() {
                let _dashboard = window.global::<DashboardLogic>();
                let _scheduler = window.global::<SchedulerLogic>();

                // Get the selected repository
                if repo_index >= 0 {
                    let (_repo_name, repo_path, repo_password) = {
                        let s = state_clone.lock().unwrap();
                        if (repo_index as usize) < s.bookmarks.len() {
                            let bm = &s.bookmarks[repo_index as usize];
                            let pwd = s.session_passwords.get(&bm.path).cloned();
                            (bm.name.clone(), bm.path.clone(), pwd)
                        } else {
                            return;
                        }
                    };

                    // Spawn async task to load archives from this repository
                    let window_weak2 = window_weak.clone();
                    tokio::spawn(async move {
                        match async {
                            let storage = borg_core::storage::StorageConfig::Local {
                                path: std::path::PathBuf::from(&repo_path)
                            };
                            let op = borg_core::storage::build_operator(storage)
                                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                            let repo = borg_core::repository::Repository::open(
                                op,
                                repo_path.clone(),
                                repo_password.as_deref()
                            ).await
                                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                            let manifest = repo.load_manifest()
                                .await
                                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                            Ok::<_, anyhow::Error>(manifest.archives)
                        }.await {
                            Ok(archive_list) => {
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(w) = window_weak2.upgrade() {
                                        // Convert archive names to ArchiveEntry objects
                                        let archive_entries: Vec<super::ArchiveEntry> = archive_list
                                            .into_iter()
                                            .map(|archive| {
                                                super::ArchiveEntry {
                                                    name: archive.name.clone().into(),
                                                    date: archive.time.to_string().into(),
                                                    size: "".into(), // Not available in manifest
                                                    hostname: "".into(), // Not available in manifest
                                                    comment: "".into(), // Not available in manifest
                                                    tags: "".into(), // Not available in manifest
                                                }
                                            })
                                            .collect();

                                        let archive_names: Vec<slint::SharedString> = archive_entries
                                            .iter()
                                            .map(|a| slint::SharedString::from(a.name.as_str()))
                                            .collect();

                                        let scheduler = w.global::<SchedulerLogic>();
                                        let model = std::rc::Rc::new(slint::VecModel::from(archive_entries));
                                        scheduler.set_available_archives(model.into());

                                        let names_model = std::rc::Rc::new(slint::VecModel::from(archive_names));
                                        scheduler.set_available_archive_names(names_model.into());
                                    }
                                });
                            }
                            Err(e) => {
                                let error_msg = format!("Failed to load archives: {}", e);
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(w) = window_weak2.upgrade() {
                                        w.global::<DashboardLogic>()
                                            .set_terminal_text(error_msg.into());
                                    }
                                });
                            }
                        }
                    });
                }
            }
        }
    });

    // Generate task name when properties change
    scheduler_logic.on_generate_task_name({
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                let scheduler = window.global::<SchedulerLogic>();
                let index = scheduler.get_selected_task_index();
                if index >= 0 {
                    let mut tasks: Vec<super::ScheduledTask> = scheduler.get_tasks().iter().collect();
                    if (index as usize) < tasks.len() {
                        let task = &tasks[index as usize];
                        let new_name = generate_task_name(
                            &task.repo_name,
                            &task.archive_name,
                            scheduler.get_schedule_type_index(),
                            scheduler.get_schedule_hour(),
                            scheduler.get_schedule_minute(),
                        );
                        tasks[index as usize].task_name = new_name.into();
                        scheduler.set_tasks(std::rc::Rc::new(VecModel::from(tasks)).into());
                    }
                }
            }
        }
    });

    scheduler_logic.on_new_task({
        let window_weak = window_weak.clone();
        let state_clone = state.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                let scheduler = window.global::<SchedulerLogic>();
                let mut tasks: Vec<super::ScheduledTask> = scheduler.get_tasks().iter().collect();

                let task_name = if !tasks.is_empty() {
                    format!("new_task_{}", tasks.len())
                } else {
                    "new_task_1".to_string()
                };

                let new_task = super::ScheduledTask {
                    task_name: task_name.clone().into(),
                    repo_name: "".into(),
                    archive_name: "".into(),
                    schedule: super::BackupSchedule {
                        schedule_type: super::ScheduleType::Daily,
                        weekday: 0,
                        day_of_month: 1,
                        hour: 3,
                        minute: 0,
                        run_on_boot_if_missed: true,
                    }
                };
                tasks.push(new_task.clone());
                scheduler.set_tasks(std::rc::Rc::new(VecModel::from(tasks)).into());
                scheduler.set_selected_task_index(scheduler.get_tasks().row_count() as i32 - 1);

                // Reset form fields
                scheduler.set_schedule_type_index(0);
                scheduler.set_schedule_weekday(0);
                scheduler.set_schedule_day_of_month(1);
                scheduler.set_schedule_hour(3);
                scheduler.set_schedule_minute(0);
                scheduler.set_schedule_run_on_boot(true);

                // Persist new task
                let mut s = state_clone.lock().unwrap();
                s.scheduled_tasks.push(ScheduledTask {
                    task_name,
                    repo_name: "".to_string(),
                    archive_name: "".to_string(),
                    schedule: BackupSchedule {
                        schedule_type: ScheduleType::Daily,
                        weekday: 0,
                        day_of_month: 1,
                        hour: 3,
                        minute: 0,
                        run_on_boot_if_missed: true,
                    }
                });
                let _ = s.save_scheduled_tasks();
            }
        }
    });

    scheduler_logic.on_save_task({
        let state_clone = state.clone();
        let window_weak = window_weak.clone();
        move |task| {
            if let Some(_window) = window_weak.upgrade() {
                let rust_task = ScheduledTask {
                    task_name: task.task_name.to_string(),
                    repo_name: task.repo_name.to_string(),
                    archive_name: task.archive_name.to_string(),
                    schedule: BackupSchedule {
                        schedule_type: match task.schedule.schedule_type {
                            super::ScheduleType::Daily => ScheduleType::Daily,
                            super::ScheduleType::Weekly => ScheduleType::Weekly,
                            super::ScheduleType::Monthly => ScheduleType::Monthly,
                            super::ScheduleType::Manual => ScheduleType::Manual,
                        },
                        weekday: task.schedule.weekday,
                        day_of_month: task.schedule.day_of_month,
                        hour: task.schedule.hour,
                        minute: task.schedule.minute,
                        run_on_boot_if_missed: task.schedule.run_on_boot_if_missed,
                    }
                };

                let mut s = state_clone.lock().unwrap();
                // Find and update or add task by task_name
                if let Some(pos) = s.scheduled_tasks.iter().position(|t| t.task_name == rust_task.task_name) {
                    s.scheduled_tasks[pos] = rust_task;
                } else {
                    s.scheduled_tasks.push(rust_task);
                }
                let _ = s.save_scheduled_tasks();
            }
        }
    });

    scheduler_logic.on_delete_task({
        let state_clone = state.clone();
        let window_weak = window_weak.clone();
        move |index| {
            if let Some(window) = window_weak.upgrade() {
                if index < 0 { return; }

                // Get task name to delete
                let task_name = {
                    let scheduler = window.global::<SchedulerLogic>();
                    let tasks: Vec<super::ScheduledTask> = scheduler.get_tasks().iter().collect();
                    if (index as usize) < tasks.len() {
                        tasks[index as usize].task_name.to_string()
                    } else {
                        return;
                    }
                };

                let mut s = state_clone.lock().unwrap();
                s.scheduled_tasks.retain(|t| t.task_name != task_name);
                let _ = s.save_scheduled_tasks();

                let scheduler = window.global::<SchedulerLogic>();
                let mut tasks: Vec<super::ScheduledTask> = scheduler.get_tasks().iter().collect();
                tasks.remove(index as usize);
                scheduler.set_tasks(std::rc::Rc::new(VecModel::from(tasks)).into());
                scheduler.set_selected_task_index(-1);
            }
        }
    });
}
