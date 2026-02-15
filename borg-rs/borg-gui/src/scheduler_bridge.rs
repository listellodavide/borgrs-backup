use super::{AppState, DashboardLogic, MainWindow, SchedulerLogic};
use crate::app_state::{BorgAppState as RustAppState, ScheduledTask};
use slint::{ComponentHandle, Model, VecModel};
use std::sync::{Arc, Mutex};

// Generate task name from components
fn generate_task_name(
    repo_name: &str,
    archive_name: &str,
    schedule_type: i32,
    hour: i32,
    minute: i32,
) -> String {
    let freq = match schedule_type {
        0 => "daily",
        1 => "weekly",
        2 => "monthly",
        3 => "manual",
        _ => "manual",
    };
    format!(
        "{}_{}_{}_{:02}_{:02}",
        repo_name, archive_name, freq, hour, minute
    )
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
    let slint_repos: Vec<super::RepoItem> = repos
        .iter()
        .map(|r| super::RepoItem {
            name: r.name.clone().into(),
            path: r.path.clone().into(),
            repo_type: r.repo_type.clone().into(),
        })
        .collect();

    let repo_names: Vec<slint::SharedString> = repos
        .iter()
        .map(|r| slint::SharedString::from(r.name.as_str()))
        .collect();

    scheduler_logic
        .set_available_repositories(std::rc::Rc::new(VecModel::from(slint_repos.clone())).into());
    scheduler_logic.set_available_repo_names(std::rc::Rc::new(VecModel::from(repo_names)).into());

    // Handle repository list refresh when entering the scheduler view
    scheduler_logic.on_refresh_repositories({
        let state_clone = state.clone();
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                let scheduler = window.global::<SchedulerLogic>();

                // Reload repositories from state
                let repos = {
                    let s = state_clone.lock().unwrap();
                    s.bookmarks.clone()
                };
                let slint_repos: Vec<super::RepoItem> = repos
                    .iter()
                    .map(|r| super::RepoItem {
                        name: r.name.clone().into(),
                        path: r.path.clone().into(),
                        repo_type: r.repo_type.clone().into(),
                    })
                    .collect();

                let repo_names: Vec<slint::SharedString> = repos
                    .iter()
                    .map(|r| slint::SharedString::from(r.name.as_str()))
                    .collect();

                scheduler.set_available_repositories(
                    std::rc::Rc::new(VecModel::from(slint_repos)).into(),
                );
                scheduler
                    .set_available_repo_names(std::rc::Rc::new(VecModel::from(repo_names)).into());
            }
        }
    });
    let tasks = {
        let s = state.lock().unwrap();
        s.scheduled_tasks.clone()
    };
    let slint_tasks: Vec<super::ScheduledTask> = tasks
        .into_iter()
        .map(|t| {
            let paths: Vec<slint::SharedString> =
                t.paths_to_backup.into_iter().map(|p| p.into()).collect();
            let tags: Vec<slint::SharedString> = t.tags.as_ref().map_or(vec![], |vec| {
                vec.iter().map(|tag| slint::SharedString::from(tag.as_str())).collect()
            });
            super::ScheduledTask {
                task_name: t.task_name.into(),
                repo_name: t.repo_name.into(),
                repo_path: t.repo_path.into(),
                repo_type: t.repo_type.into(),
                archive_name: t.archive_name.into(),
                paths_to_backup: std::rc::Rc::new(slint::VecModel::from(paths)).into(),
                compression: t.compression.into(),
                comment: t.comment.unwrap_or_default().into(),
                tags: std::rc::Rc::new(slint::VecModel::from(tags)).into(),
                schedule: super::BackupSchedule {
                    schedule_type: match t.schedule_type.as_str() {
                        "Daily" => super::ScheduleType::Daily,
                        "Weekly" => super::ScheduleType::Weekly,
                        "Monthly" => super::ScheduleType::Monthly,
                        _ => super::ScheduleType::Manual,
                    },
                    weekday: t.weekday,
                    day_of_month: t.day_of_month,
                    hour: t.hour,
                    minute: t.minute,
                    run_on_boot_if_missed: t.run_on_boot_if_missed,
                },
                execution_count: t.execution_count as i32,
                active: t.active,
                last_run: t.last_run.unwrap_or_else(|| "Never".to_string()).into(),
            }
        })
        .collect();
    scheduler_logic.set_tasks(std::rc::Rc::new(VecModel::from(slint_tasks)).into());

    scheduler_logic.on_on_selected_task_index_changed({
        let window_weak = window_weak.clone();
        move |index| {
            if let Some(window) = window_weak.upgrade() {
                let scheduler = window.global::<SchedulerLogic>();
                scheduler.set_selected_task_index(index);
                if index >= 0 {
                    let tasks: Vec<super::ScheduledTask> = scheduler.get_tasks().iter().collect();
                    if let Some(task) = tasks.get(index as usize) {
                        scheduler.set_selected_task_name(task.task_name.clone());
                        scheduler.set_selected_task_repo_name(task.repo_name.clone());
                        scheduler.set_selected_task_repo_path(task.repo_path.clone());
                        scheduler.set_selected_task_repo_type(task.repo_type.clone());
                        scheduler.set_selected_task_archive_name(task.archive_name.clone());
                        scheduler.set_selected_task_schedule_type_index(
                            match task.schedule.schedule_type {
                                super::ScheduleType::Daily => 0,
                                super::ScheduleType::Weekly => 1,
                                super::ScheduleType::Monthly => 2,
                                super::ScheduleType::Manual => 3,
                            },
                        );
                        scheduler.set_selected_task_weekday(task.schedule.weekday);
                        scheduler.set_selected_task_day_of_month(task.schedule.day_of_month);
                        scheduler.set_selected_task_hour(task.schedule.hour);
                        scheduler.set_selected_task_minute(task.schedule.minute);
                        scheduler
                            .set_selected_task_run_on_boot(task.schedule.run_on_boot_if_missed);
                        scheduler.set_selected_task_execution_count(task.execution_count);
                        scheduler.set_selected_task_active(task.active);
                    }
                }
            }
        }
    });

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
                    let (_repo_name, repo_path, repo_type, repo_password) = {
                        let s = state_clone.lock().unwrap();
                        if (repo_index as usize) < s.bookmarks.len() {
                            let bm = &s.bookmarks[repo_index as usize];
                            // Try session passwords first (keyed by path)
                            let pwd = s
                                .session_passwords
                                .get(&bm.path)
                                .cloned()
                                .or_else(|| s.get_password(&bm.name).ok()); // Then keyring (keyed by name)
                            (bm.name.clone(), bm.path.clone(), bm.repo_type.clone(), pwd)
                        } else {
                            return;
                        }
                    };

                    // Set the repo_path in the scheduler logic
                    if let Some(window) = window_weak.upgrade() {
                        let scheduler = window.global::<SchedulerLogic>();
                        scheduler.set_selected_task_repo_path(repo_path.clone().into());
                        scheduler.set_selected_task_repo_type(repo_type.into());
                    }

                    // Spawn async task to load archives from this repository
                    let window_weak2 = window_weak.clone();
                    tokio::spawn(async move {
                        match crate::commands::list_archives(&repo_path, repo_password.as_deref())
                            .await
                        {
                            Ok(archive_list) => {
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(w) = window_weak2.upgrade() {
                                        let archive_names: Vec<slint::SharedString> = archive_list
                                             .into_iter()
                                             .map(|name| name.into())
                                             .collect();

                                        let scheduler = w.global::<SchedulerLogic>();
                                        let names_model =
                                            std::rc::Rc::new(slint::VecModel::from(archive_names));
                                        scheduler.set_available_archive_names(names_model.into());
                                    }
                                });
                            }
                            Err(e) => {
                                let error_msg = format!("Failed to load archives: {}", e);
                                println!("{}", error_msg); // Log to console
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(w) = window_weak2.upgrade() {
                                        // Optionally show error in UI, but maybe not block the user
                                        // w.global::<DashboardLogic>().set_terminal_text(error_msg.into());

                                        // Clear the list on error
                                        let scheduler = w.global::<SchedulerLogic>();
                                        let names_model =
                                            std::rc::Rc::new(slint::VecModel::from(Vec::<
                                                slint::SharedString,
                                            >::new(
                                            )));
                                        scheduler.set_available_archive_names(names_model.into());
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
                let new_name = generate_task_name(
                    &scheduler.get_selected_task_repo_name(),
                    &scheduler.get_selected_task_archive_name(),
                    scheduler.get_selected_task_schedule_type_index(),
                    scheduler.get_selected_task_hour(),
                    scheduler.get_selected_task_minute(),
                );
                scheduler.set_selected_task_name(new_name.into());
            }
        }
    });

    scheduler_logic.on_new_task({
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                println!("Creating new scheduled task");
                let scheduler = window.global::<SchedulerLogic>();

                // Refresh repositories before creating new task
                scheduler.invoke_refresh_repositories();

                let new_task_name = generate_task_name("new", "task", 0, 3, 0);
                let new_task = super::ScheduledTask {
                    task_name: new_task_name.into(),
                    repo_name: "".into(),
                    repo_path: "".into(),
                    repo_type: "local".into(),
                    archive_name: "".into(),
                    paths_to_backup: std::rc::Rc::new(slint::VecModel::from(vec![])).into(),
                    compression: "zstd,3".into(),
                    comment: "".into(),
                    tags: std::rc::Rc::new(slint::VecModel::from(vec![])).into(),
                    schedule: super::BackupSchedule {
                        schedule_type: super::ScheduleType::Daily,
                        weekday: 0,
                        day_of_month: 1,
                        hour: 3,
                        minute: 0,
                        run_on_boot_if_missed: true,
                    },
                    execution_count: 0,
                    active: true,
                    last_run: "Never".into(),
                };
                let mut tasks: Vec<super::ScheduledTask> = scheduler.get_tasks().iter().collect();
                tasks.push(new_task);
                scheduler.set_tasks(std::rc::Rc::new(VecModel::from(tasks)).into());
                let new_index = scheduler.get_tasks().row_count() as i32 - 1;
                scheduler.invoke_on_selected_task_index_changed(new_index);
            }
        }
    });

    scheduler_logic.on_save_task({
        let state_clone = state.clone();
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                let scheduler = window.global::<SchedulerLogic>();

                // Get the last_run from the existing task if it exists
                let last_run = {
                    let tasks: Vec<super::ScheduledTask> = scheduler.get_tasks().iter().collect();
                    let index = scheduler.get_selected_task_index() as usize;
                    if index < tasks.len() {
                        tasks[index].last_run.clone()
                    } else {
                        "Never".into()
                    }
                };

                let is_active = scheduler.get_selected_task_active();
                let repo_name = scheduler.get_selected_task_repo_name().to_string();
                let repo_path = scheduler.get_selected_task_repo_path().to_string();
                let repo_type = scheduler.get_selected_task_repo_type().to_string();
                let archive_name = scheduler.get_selected_task_archive_name().to_string();
                let task_name = scheduler.get_selected_task_name().to_string();
                let schedule_type_index = scheduler.get_selected_task_schedule_type_index();
                let weekday = scheduler.get_selected_task_weekday();
                let day_of_month = scheduler.get_selected_task_day_of_month();
                let hour = scheduler.get_selected_task_hour();
                let minute = scheduler.get_selected_task_minute();
                let run_on_boot = scheduler.get_selected_task_run_on_boot();
                let execution_count = scheduler.get_selected_task_execution_count();

                let pending_action = window.global::<AppState>().get_pending_auth_action().to_string();
                if pending_action != "save_scheduled_task" {
                    let mut s = state_clone.lock().unwrap();
                    s.auth_retry_count = 0;
                }

                let state_arc = state_clone.clone();
                let window_weak_save = window_weak.clone();

                tokio::spawn(async move {
                    // Try to fetch paths from archive metadata if archive is selected
                    let paths;
                    let compression;

                    let (repo_password, default_paths, default_comp) = {
                        let s = state_arc.lock().unwrap();
                        let pwd = s.session_passwords.get(&repo_path).cloned()
                            .or_else(|| s.get_password(&repo_name).ok());
                        if let Some(bookmark) = s.get_archive_bookmark_for_repo(&repo_path) {
                            (pwd, bookmark.paths.clone(), bookmark.compression.clone())
                        } else {
                            (pwd, vec![], Some("zstd,3".to_string()))
                        }
                    };

                    if !archive_name.is_empty() {
                        println!(
                            "Fetching paths for archive '{}' in repo '{}'",
                            archive_name, repo_path
                        );
                        match crate::commands::get_archive_paths(
                            &repo_path,
                            repo_password.as_deref(),
                            &archive_name,
                        )
                        .await
                        {
                            Ok(fetched_paths) => {
                                {
                                    let mut s = state_arc.lock().unwrap();
                                    s.auth_retry_count = 0;
                                }
                                paths = fetched_paths;
                            }
                            Err(e) => {
                                let err_msg = e.to_string();
                                if err_msg.contains("Invalid passphrase")
                                    || err_msg.contains("Passphrase required")
                                {
                                    let (retry_count, should_retry) = {
                                        let mut s = state_arc.lock().unwrap();
                                        s.auth_retry_count += 1;
                                        (s.auth_retry_count, s.auth_retry_count < 3)
                                    };

                                    if should_retry {
                                        let _ = slint::invoke_from_event_loop(move || {
                                            if let Some(w) = window_weak_save.upgrade() {
                                                let app = w.global::<AppState>();
                                                app.set_pending_auth_action(
                                                    "save_scheduled_task".into(),
                                                );
                                                app.set_pending_auth_repo_path(repo_path.clone().into());
                                                app.set_show_password_dialog(true);
                                                let msg = format!(
                                                    "Authentication failed (attempt {}/3). Please enter passphrase for {}:",
                                                    retry_count, repo_path
                                                );
                                                app.set_password_dialog_message(msg.into());
                                            }
                                        });
                                        return; // Stop and wait for password
                                    } else {
                                        eprintln!("Authentication failed after 3 attempts: {}", e);
                                        // Reset retry count and fail
                                        {
                                            let mut s = state_arc.lock().unwrap();
                                            s.auth_retry_count = 0;
                                        }
                                        paths = default_paths;
                                    }
                                } else {
                                    eprintln!("Error fetching archive paths: {}", e);
                                    paths = default_paths;
                                }
                            }
                        }
                    } else {
                        paths = default_paths;
                    }
                    compression = default_comp;

                    let rust_task = ScheduledTask {
                        id: None,
                        task_name: task_name.clone(),
                        repo_name: repo_name.clone(),
                        repo_path: repo_path.clone(),
                        repo_type: repo_type.clone(),
                        archive_name: archive_name.clone(),
                        paths_to_backup: paths.clone(),
                        compression: compression.unwrap_or_else(|| "zstd,3".to_string()),
                        comment: None,
                        tags: None,
                        schedule_type: match schedule_type_index {
                             0 => "Daily".to_string(),
                             1 => "Weekly".to_string(),
                             2 => "Monthly".to_string(),
                             _ => "Manual".to_string(),
                         },
                        weekday,
                        day_of_month,
                        hour,
                        minute,
                        run_on_boot_if_missed: run_on_boot,
                        execution_count: execution_count as u32,
                        active: is_active,
                        last_run: if last_run == "Never" {
                            None
                        } else {
                            Some(last_run.to_string())
                        },
                     };

                    println!(
                        "Saving task: {} with {} paths",
                        rust_task.task_name,
                        rust_task.paths_to_backup.len()
                    );
                    let _ = RustAppState::save_task_async(state_arc.clone(), rust_task).await;

                    // Request scheduler to reload
                    borg_core::scheduler::Scheduler::request_reload();

                    // Update the UI model from the event loop
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window_weak_save.upgrade() {
                            let scheduler = window.global::<SchedulerLogic>();
                            let mut tasks: Vec<super::ScheduledTask> =
                                scheduler.get_tasks().iter().collect();
                            let index = scheduler.get_selected_task_index() as usize;

                            if index < tasks.len() {
                                let slint_paths: Vec<slint::SharedString> =
                                    paths.into_iter().map(|p| p.into()).collect();
                                tasks[index] = super::ScheduledTask {
                                    repo_type: repo_type.into(),
                                    task_name: task_name.into(),
                                    repo_name: repo_name.into(),
                                    repo_path: repo_path.into(),
                                    archive_name: archive_name.into(),
                                    paths_to_backup: std::rc::Rc::new(slint::VecModel::from(
                                        slint_paths,
                                    ))
                                    .into(),
                                    compression: "zstd,3".into(),
                                    comment: "".into(),
                                    tags: std::rc::Rc::new(slint::VecModel::from(vec![])).into(),
                                    schedule: super::BackupSchedule {
                                        schedule_type: match schedule_type_index {
                                            0 => super::ScheduleType::Daily,
                                            1 => super::ScheduleType::Weekly,
                                            2 => super::ScheduleType::Monthly,
                                            _ => super::ScheduleType::Manual,
                                        },
                                        weekday,
                                        day_of_month,
                                        hour,
                                        minute,
                                        run_on_boot_if_missed: run_on_boot,
                                    },
                                    execution_count,
                                    active: is_active,
                                    last_run: last_run,
                                };
                                scheduler.set_tasks(std::rc::Rc::new(VecModel::from(tasks)).into());
                            }
                            // Close the form
                            scheduler.set_selected_task_index(-1);
                        }
                    });
                });
            }
        }
    });

    scheduler_logic.on_delete_task({
        let state_clone = state.clone();
        let window_weak = window_weak.clone();
        move |index| {
            if let Some(window) = window_weak.upgrade() {
                if index < 0 {
                    return;
                }

                let task_name = {
                    let scheduler = window.global::<SchedulerLogic>();
                    let tasks: Vec<super::ScheduledTask> = scheduler.get_tasks().iter().collect();
                    if (index as usize) < tasks.len() {
                        tasks[index as usize].task_name.to_string()
                    } else {
                        return;
                    }
                };

                println!("Deleting task: {}", task_name);

                let state_arc = state_clone.clone();
                tokio::spawn(async move {
                    let task_id = {
                        let s = state_arc.lock().unwrap();
                        s.scheduled_tasks
                            .iter()
                            .find(|t| t.task_name == task_name)
                            .and_then(|t| t.id)
                    };

                    if let Some(id) = task_id {
                        let _ = RustAppState::delete_task_async(state_arc, id).await;
                        // Request scheduler reload
                        borg_core::scheduler::Scheduler::request_reload();
                    }
                });

                let scheduler = window.global::<SchedulerLogic>();
                let mut tasks: Vec<super::ScheduledTask> = scheduler.get_tasks().iter().collect();
                tasks.remove(index as usize);
                scheduler.set_tasks(std::rc::Rc::new(VecModel::from(tasks)).into());
                scheduler.set_selected_task_index(-1);
            }
        }
    });

    scheduler_logic.on_toggle_task_active({
        let state_clone = state.clone();
        let window_weak = window_weak.clone();
        move |index, active| {
            if let Some(window) = window_weak.upgrade() {
                if index < 0 {
                    return;
                }

                let scheduler = window.global::<SchedulerLogic>();
                let mut tasks: Vec<super::ScheduledTask> = scheduler.get_tasks().iter().collect();

                if (index as usize) < tasks.len() {
                    // Update UI model
                    tasks[index as usize].active = active;
                    scheduler.set_tasks(std::rc::Rc::new(VecModel::from(tasks)).into());

                    // Update Rust state
                    let task_name = scheduler
                        .get_tasks()
                        .row_data(index as usize)
                        .unwrap()
                        .task_name
                        .to_string();
                    let state_arc = state_clone.clone();
                    tokio::spawn(async move {
                        let task = {
                            let s = state_arc.lock().unwrap();
                            s.scheduled_tasks
                                .iter()
                                .find(|t| t.task_name == task_name)
                                .cloned()
                        };

                        if let Some(mut t) = task {
                            t.active = active;
                            let _ = RustAppState::save_task_async(state_arc, t).await;
                            // Request scheduler to reload when active status changes
                            borg_core::scheduler::Scheduler::request_reload();
                        }
                    });
                }
            }
        }
    });
}
