use std::sync::{Arc, Mutex};
use std::time::Duration;
use chrono::Local;
use crate::app_state::BorgAppState;

pub fn start_task_runner(app_state: Arc<Mutex<BorgAppState>>) {
    tokio::spawn(async move {
        loop {
            let now = Local::now();
            let tasks_to_run = {
                let state = app_state.lock().unwrap();
                state.scheduled_tasks.iter().filter(|task| {
                    // TODO: Implement scheduling logic
                    false
                }).cloned().collect::<Vec<_>>()
            };

            for task in tasks_to_run {
                // TODO: Implement task execution
                println!("Running task: {}", task.task_name);
            }

            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}
