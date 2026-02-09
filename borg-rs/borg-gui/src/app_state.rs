use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
pub struct AppState {
    pub current_repo: Option<String>,
    pub is_processing: bool,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }
}
