use std::sync::{Arc, Mutex};
use slint::{ComponentHandle, Model, VecModel};
use crate::app_state::{BorgAppState as RustAppState, ScheduledTask, BackupSchedule, ScheduleType};
use super::{MainWindow, SchedulerLogic};

pub fn init_scheduler_bridge(window: &MainWindow, state: Arc<Mutex<RustAppState>>) {
    let scheduler_logic = window.global::<SchedulerLogic>();
    let window_weak = window.as_weak();

    // Load initial tasks
    let tasks = {
        let s = state.lock().unwrap();
        s.scheduled_tasks.clone()
    };
    let slint_tasks: Vec<super::ScheduledTask> = tasks.into_iter().map(|t| {
        super::ScheduledTask {
            repo_name: t.repo_name.into(),
            archive_name: t.archive_name.into(),
            schedule: super::BackupSchedule {
                schedule_type: match t.schedule.schedule_type {
                    ScheduleType::Daily => super::ScheduleType::Daily,
                    ScheduleType::Weekly => super::ScheduleType::Weekly,
                    ScheduleType::Manual => super::ScheduleType::Manual,
                },
                weekday: t.schedule.weekday,
                hour: t.schedule.hour,
                minute: t.schedule.minute,
                run_on_boot_if_missed: t.schedule.run_on_boot_if_missed,
            }
        }
    }).collect();
    scheduler_logic.set_tasks(std::rc::Rc::new(VecModel::from(slint_tasks)).into());

    scheduler_logic.on_new_task({
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                let scheduler = window.global::<SchedulerLogic>();
                let mut tasks: Vec<super::ScheduledTask> = scheduler.get_tasks().iter().collect();
                tasks.push(super::ScheduledTask {
                    repo_name: "New Task".into(),
                    archive_name: "archive".into(),
                    schedule: super::BackupSchedule {
                        schedule_type: super::ScheduleType::Daily,
                        weekday: 0,
                        hour: 3,
                        minute: 0,
                        run_on_boot_if_missed: true,
                    }
                });
                scheduler.set_tasks(std::rc::Rc::new(VecModel::from(tasks)).into());
                scheduler.set_selected_task_index(scheduler.get_tasks().row_count() as i32 - 1);
            }
        }
    });

    scheduler_logic.on_save_task({
        let state_clone = state.clone();
        let window_weak = window_weak.clone();
        move |task| {
            if let Some(window) = window_weak.upgrade() {
                let scheduler = window.global::<SchedulerLogic>();
                let index = scheduler.get_selected_task_index();
                if index < 0 { return; }

                let rust_task = ScheduledTask {
                    repo_name: task.repo_name.to_string(),
                    archive_name: task.archive_name.to_string(),
                    schedule: BackupSchedule {
                        schedule_type: match task.schedule.schedule_type {
                            super::ScheduleType::Daily => ScheduleType::Daily,
                            super::ScheduleType::Weekly => ScheduleType::Weekly,
                            super::ScheduleType::Manual => ScheduleType::Manual,
                        },
                        weekday: task.schedule.weekday,
                        hour: task.schedule.hour,
                        minute: task.schedule.minute,
                        run_on_boot_if_missed: task.schedule.run_on_boot_if_missed,
                    }
                };

                let mut s = state_clone.lock().unwrap();
                if (index as usize) < s.scheduled_tasks.len() {
                    s.scheduled_tasks[index as usize] = rust_task;
                } else {
                    s.scheduled_tasks.push(rust_task);
                }
                let _ = s.save_scheduled_tasks();

                // Update UI
                let mut tasks: Vec<super::ScheduledTask> = scheduler.get_tasks().iter().collect();
                tasks[index as usize] = task;
                scheduler.set_tasks(std::rc::Rc::new(VecModel::from(tasks)).into());
            }
        }
    });

    scheduler_logic.on_delete_task({
        let state_clone = state.clone();
        let window_weak = window_weak.clone();
        move |index| {
            if let Some(window) = window_weak.upgrade() {
                if index < 0 { return; }

                let mut s = state_clone.lock().unwrap();
                if (index as usize) < s.scheduled_tasks.len() {
                    s.scheduled_tasks.remove(index as usize);
                    let _ = s.save_scheduled_tasks();
                }

                let scheduler = window.global::<SchedulerLogic>();
                let mut tasks: Vec<super::ScheduledTask> = scheduler.get_tasks().iter().collect();
                tasks.remove(index as usize);
                scheduler.set_tasks(std::rc::Rc::new(VecModel::from(tasks)).into());
                scheduler.set_selected_task_index(-1);
            }
        }
    });
}
