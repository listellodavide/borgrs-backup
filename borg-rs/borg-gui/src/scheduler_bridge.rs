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
            },
            execution_count: t.execution_count,
            active: t.active,
            last_run: t.last_run.into(),
        }
    }).collect();
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
                        scheduler.set_selected_task_archive_name(task.archive_name.clone());
                        scheduler.set_selected_task_schedule_type_index(match task.schedule.schedule_type {
                            super::ScheduleType::Daily => 0,
                            super::ScheduleType::Weekly => 1,
                            super::ScheduleType::Monthly => 2,
                            super::ScheduleType::Manual => 3,
                        });
                        scheduler.set_selected_task_weekday(task.schedule.weekday);
                        scheduler.set_selected_task_day_of_month(task.schedule.day_of_month);
                        scheduler.set_selected_task_hour(task.schedule.hour);
                        scheduler.set_selected_task_minute(task.schedule.minute);
                        scheduler.set_selected_task_run_on_boot(task.schedule.run_on_boot_if_missed);
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
                        match crate::commands::list_archives(&repo_path, repo_password.as_deref()).await {
                            Ok(archive_list) => {
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(w) = window_weak2.upgrade() {
                                        let archive_names: Vec<slint::SharedString> = archive_list
                                            .into_iter()
                                            .map(|name| name.into())
                                            .collect();

                                        let scheduler = w.global::<SchedulerLogic>();
                                        let names_model = std::rc::Rc::new(slint::VecModel::from(archive_names));
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
                                        let names_model = std::rc::Rc::new(slint::VecModel::from(Vec::<slint::SharedString>::new()));
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
                let new_task_name = generate_task_name("new", "task", 0, 3, 0);
                let new_task = super::ScheduledTask {
                    task_name: new_task_name.into(),
                    repo_name: "".into(),
                    archive_name: "".into(),
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

                let rust_task = ScheduledTask {
                    task_name: scheduler.get_selected_task_name().to_string(),
                    repo_name: scheduler.get_selected_task_repo_name().to_string(),
                    archive_name: scheduler.get_selected_task_archive_name().to_string(),
                    schedule: BackupSchedule {
                        schedule_type: match scheduler.get_selected_task_schedule_type_index() {
                            0 => ScheduleType::Daily,
                            1 => ScheduleType::Weekly,
                            2 => ScheduleType::Monthly,
                            _ => ScheduleType::Manual,
                        },
                        weekday: scheduler.get_selected_task_weekday(),
                        day_of_month: scheduler.get_selected_task_day_of_month(),
                        hour: scheduler.get_selected_task_hour(),
                        minute: scheduler.get_selected_task_minute(),
                        run_on_boot_if_missed: scheduler.get_selected_task_run_on_boot(),
                    },
                    execution_count: scheduler.get_selected_task_execution_count(),
                    active: scheduler.get_selected_task_active(),
                    last_run: last_run.to_string(),
                };

                println!("Saving task: {}", rust_task.task_name);

                let mut s = state_clone.lock().unwrap();
                if let Some(pos) = s.scheduled_tasks.iter().position(|t| t.task_name == rust_task.task_name) {
                    s.scheduled_tasks[pos] = rust_task.clone();
                } else {
                    s.scheduled_tasks.push(rust_task.clone());
                }
                let _ = s.save_scheduled_tasks();

                // Update the UI model
                let mut tasks: Vec<super::ScheduledTask> = scheduler.get_tasks().iter().collect();
                let index = scheduler.get_selected_task_index() as usize;
                if index < tasks.len() {
                    tasks[index] = super::ScheduledTask {
                        task_name: scheduler.get_selected_task_name(),
                        repo_name: scheduler.get_selected_task_repo_name(),
                        archive_name: scheduler.get_selected_task_archive_name(),
                        schedule: super::BackupSchedule {
                            schedule_type: match scheduler.get_selected_task_schedule_type_index() {
                                0 => super::ScheduleType::Daily,
                                1 => super::ScheduleType::Weekly,
                                2 => super::ScheduleType::Monthly,
                                _ => super::ScheduleType::Manual,
                            },
                            weekday: scheduler.get_selected_task_weekday(),
                            day_of_month: scheduler.get_selected_task_day_of_month(),
                            hour: scheduler.get_selected_task_hour(),
                            minute: scheduler.get_selected_task_minute(),
                            run_on_boot_if_missed: scheduler.get_selected_task_run_on_boot(),
                        },
                        execution_count: scheduler.get_selected_task_execution_count(),
                        active: scheduler.get_selected_task_active(),
                        last_run: last_run,
                    };
                    scheduler.set_tasks(std::rc::Rc::new(VecModel::from(tasks)).into());
                }

                // Close the form
                scheduler.set_selected_task_index(-1);
            }
        }
    });

    scheduler_logic.on_delete_task({
        let state_clone = state.clone();
        let window_weak = window_weak.clone();
        move |index| {
            if let Some(window) = window_weak.upgrade() {
                if index < 0 { return; }

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

    scheduler_logic.on_toggle_task_active({
        let state_clone = state.clone();
        let window_weak = window_weak.clone();
        move |index, active| {
            if let Some(window) = window_weak.upgrade() {
                if index < 0 { return; }

                let scheduler = window.global::<SchedulerLogic>();
                let mut tasks: Vec<super::ScheduledTask> = scheduler.get_tasks().iter().collect();

                if (index as usize) < tasks.len() {
                    // Update UI model
                    tasks[index as usize].active = active;
                    scheduler.set_tasks(std::rc::Rc::new(VecModel::from(tasks)).into());

                    // Update Rust state
                    let task_name = scheduler.get_tasks().row_data(index as usize).unwrap().task_name.to_string();
                    let mut s = state_clone.lock().unwrap();
                    if let Some(pos) = s.scheduled_tasks.iter().position(|t| t.task_name == task_name) {
                        s.scheduled_tasks[pos].active = active;
                        let _ = s.save_scheduled_tasks();
                    }
                }
            }
        }
    });
}
