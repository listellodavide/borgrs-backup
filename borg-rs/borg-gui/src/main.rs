slint::include_modules!();

pub mod app_state;
pub mod commands;
pub mod bridge;

use std::sync::{Arc, Mutex};
use app_state::BorgAppState as RustAppState;
use slint::ComponentHandle;

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

        let archives = vec![
            ArchiveEntry { 
                name: "daily-2023-11-20-0001".into(), 
                date: "2023-11-20 00:01".into(), 
                size: "1.2 GB".into(),
                hostname: "backup-node-1".into(),
                comment: "Regular daily backup".into(),
                tags: "daily,prod".into(),
            },
            ArchiveEntry { 
                name: "daily-2023-11-19-0001".into(), 
                date: "2023-11-19 00:01".into(), 
                size: "1.1 GB".into(),
                hostname: "backup-node-1".into(),
                comment: "Regular daily backup".into(),
                tags: "daily,prod".into(),
            },
        ];
        
        let repos_model = std::rc::Rc::new(slint::VecModel::from(repos));
        dashboard.set_repositories(repos_model.into());
        
        let archives_model = std::rc::Rc::new(slint::VecModel::from(archives));
        dashboard.set_archives(archives_model.into());
        
        dashboard.set_terminal_text("Welcome to Borg-GUI. Ready.".into());
    }

    bridge::init_bridge(&main_window, rust_app_state.clone());

    main_window.run()?;
    Ok(())
}
