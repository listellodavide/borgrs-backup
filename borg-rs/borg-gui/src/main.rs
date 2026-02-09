slint::include_modules!();

mod app_state;
mod commands;
mod bridge;

use std::sync::{Arc, Mutex};
use app_state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let main_window = MainWindow::new()?;
    let app_state = Arc::new(Mutex::new(AppState::new()));

    bridge::init_bridge(&main_window, app_state.clone());

    main_window.run()?;
    Ok(())
}
