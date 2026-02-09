use crate::commands;
use crate::app_state::AppState;
use std::sync::{Arc, Mutex};
use crate::MainWindow;
use crate::DashboardLogic;
use slint::ComponentHandle;

pub fn init_bridge(window: &MainWindow, state: Arc<Mutex<AppState>>) {
    let window_weak = window.as_weak();

    // Example: Connect DashboardLogic callbacks here
    // window.global::<DashboardLogic>().on_create_new_archive(move || {
    //     let state = state.lock().unwrap();
    //     if let Some(repo) = &state.current_repo {
    //         let repo_clone = repo.clone();
    //         tokio::spawn(async move {
    //             let _ = commands::create_archive(&repo_clone).await;
    //         });
    //     }
    // });
}
