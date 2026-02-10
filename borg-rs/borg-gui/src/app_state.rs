use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "borg-gui";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoBookmark {
    pub name: String,
    pub path: String,
    pub repo_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArchiveBookmark {
    pub repo_name: String,
    pub repo_path: String,
    pub compression: String,
    pub redundancy: String,
    pub use_custom_name: bool,
    pub archive_name: Option<String>,
    pub comment: Option<String>,
    pub tags: Option<String>, // comma separated in UI
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ScheduleType {
    Daily,
    Weekly,
    Monthly,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackupSchedule {
    pub schedule_type: ScheduleType,
    pub weekday: i32,      // 0 = Mon … 6 = Sun
    pub day_of_month: i32, // 1-31 for monthly
    pub hour: i32,
    pub minute: i32,
    pub run_on_boot_if_missed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScheduledTask {
    pub task_name: String, // Auto-generated: repo_archive_frequency_hh_mm
    pub repo_name: String,
    pub archive_name: String,
    pub schedule: BackupSchedule,
    pub execution_count: i32,
    #[serde(default = "default_active")]
    pub active: bool,
    #[serde(default = "default_last_run")]
    pub last_run: String,
}

fn default_active() -> bool {
    true
}

fn default_last_run() -> String {
    "Never".to_string()
}

#[derive(Debug, Default)]
pub struct BorgAppState {
    pub current_repo: Option<String>,
    pub is_processing: bool,
    pub bookmarks: Vec<RepoBookmark>,
    pub session_passwords: HashMap<String, String>, // repo_path -> password
    pub archive_bookmarks: Vec<ArchiveBookmark>,
    pub scheduled_tasks: Vec<ScheduledTask>,

    pub archive_cache: Option<(String, Vec<crate::commands::FileEntry>)>,
}

impl BorgAppState {
    pub fn new() -> Self {
        let mut state = Self::default();
        state.load_bookmarks();
        state.load_archive_bookmarks();
        state.load_scheduled_tasks();
        state
    }

    pub fn get_bookmarks_path() -> PathBuf {
        let mut path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
        path.pop();
        path.push("repo_bookmarks.json");
        path
    }

    pub fn get_archive_bookmarks_path() -> PathBuf {
        let mut path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
        path.pop();
        path.push("archive_bookmarks.json");
        path
    }

    pub fn get_scheduled_tasks_path() -> PathBuf {
        let mut path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
        path.pop();
        path.push("scheduled_task.json");
        path
    }

    pub fn load_bookmarks(&mut self) {
        let path = Self::get_bookmarks_path();
        if path.exists() {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(bookmarks) = serde_json::from_str(&content) {
                    self.bookmarks = bookmarks;
                }
            }
        } else {
            self.bookmarks = Vec::new();
            let _ = self.save_bookmarks();
        }
    }

    pub fn save_bookmarks(&self) -> anyhow::Result<()> {
        let path = Self::get_bookmarks_path();
        let content = serde_json::to_string_pretty(&self.bookmarks)?;
        fs::write(path, content)?;
        Ok(())
    }

    pub fn load_archive_bookmarks(&mut self) {
        let path = Self::get_archive_bookmarks_path();
        if path.exists() {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(bm) = serde_json::from_str(&content) {
                    self.archive_bookmarks = bm;
                    return;
                }
            }
        }
        self.archive_bookmarks = Vec::new();
        let _ = self.save_archive_bookmarks();
    }

    pub fn save_archive_bookmarks(&self) -> anyhow::Result<()> {
        let path = Self::get_archive_bookmarks_path();
        let content = serde_json::to_string_pretty(&self.archive_bookmarks)?;
        fs::write(path, content)?;
        Ok(())
    }

    pub fn load_scheduled_tasks(&mut self) {
        let path = Self::get_scheduled_tasks_path();
        if path.exists() {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(tasks) = serde_json::from_str(&content) {
                    self.scheduled_tasks = tasks;
                }
            }
        } else {
            self.scheduled_tasks = Vec::new();
            let _ = self.save_scheduled_tasks();
        }
    }

    pub fn save_scheduled_tasks(&self) -> anyhow::Result<()> {
        let path = Self::get_scheduled_tasks_path();
        let content = serde_json::to_string_pretty(&self.scheduled_tasks)?;
        fs::write(path, content)?;
        Ok(())
    }

    pub fn upsert_archive_bookmark(&mut self, bookmark: ArchiveBookmark) {
        if let Some(existing) = self
            .archive_bookmarks
            .iter_mut()
            .find(|b| b.repo_path == bookmark.repo_path)
        {
            *existing = bookmark;
        } else {
            self.archive_bookmarks.push(bookmark);
        }
        let _ = self.save_archive_bookmarks();
    }

    pub fn get_archive_bookmark_for_repo(&self, repo_path: &str) -> Option<ArchiveBookmark> {
        self.archive_bookmarks
            .iter()
            .find(|b| b.repo_path == repo_path)
            .cloned()
    }

    pub fn add_bookmark(&mut self, bookmark: RepoBookmark, password: Option<String>) {
        if let Some(pwd) = password {
            self.session_passwords
                .insert(bookmark.path.clone(), pwd.clone());
            let _ = self.store_password(&bookmark.path, &pwd);
        }
        // Avoid duplicates by path
        if !self.bookmarks.iter().any(|b| b.path == bookmark.path) {
            self.bookmarks.push(bookmark);
            let _ = self.save_bookmarks();
        }
    }

    pub fn store_password(&self, repo_path: &str, password: &str) -> anyhow::Result<()> {
        let entry = Entry::new(KEYRING_SERVICE, repo_path)?;
        entry.set_password(password)?;
        Ok(())
    }

    pub fn get_password(&self, repo_path: &str) -> anyhow::Result<String> {
        let entry = Entry::new(KEYRING_SERVICE, repo_path)?;
        let password = entry.get_password()?;
        Ok(password)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn save_scheduled_tasks_to_path(
        tasks: &[ScheduledTask],
        path: &std::path::Path,
    ) -> anyhow::Result<()> {
        let content = serde_json::to_string_pretty(tasks)?;
        fs::write(path, content)?;
        Ok(())
    }

    fn load_scheduled_tasks_from_path(path: &std::path::Path) -> Vec<ScheduledTask> {
        if path.exists() {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(tasks) = serde_json::from_str(&content) {
                    return tasks;
                }
            }
        }
        Vec::new()
    }

    #[test]
    fn test_scheduler_state_persistence() {
        let dir = tempdir().unwrap();
        let scheduled_tasks_path = dir.path().join("scheduled_task.json");

        let tasks = vec![ScheduledTask {
            task_name: "test_task_1".to_string(),
            repo_name: "repo1".to_string(),
            archive_name: "archive1".to_string(),
            schedule: BackupSchedule {
                schedule_type: ScheduleType::Daily,
                weekday: 0,
                day_of_month: 0,
                hour: 1,
                minute: 0,
                run_on_boot_if_missed: true,
            },
            execution_count: 0,
            active: true,
            last_run: "Never".to_string(),
        }];

        let save_result = save_scheduled_tasks_to_path(&tasks, &scheduled_tasks_path);
        assert!(save_result.is_ok());

        let loaded_tasks = load_scheduled_tasks_from_path(&scheduled_tasks_path);

        assert_eq!(loaded_tasks.len(), 1);
        assert_eq!(loaded_tasks[0], tasks[0]);

        // Test deletion
        let updated_tasks = vec![];
        let save_result = save_scheduled_tasks_to_path(&updated_tasks, &scheduled_tasks_path);
        assert!(save_result.is_ok());

        let final_tasks = load_scheduled_tasks_from_path(&scheduled_tasks_path);
        assert!(final_tasks.is_empty());
    }
}
