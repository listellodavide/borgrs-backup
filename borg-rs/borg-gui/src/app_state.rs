use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

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

#[derive(Debug, Default)]
pub struct BorgAppState {
    pub current_repo: Option<String>,
    pub is_processing: bool,
    pub bookmarks: Vec<RepoBookmark>,
    pub session_passwords: HashMap<String, String>, // repo_path -> password
    pub archive_bookmarks: Vec<ArchiveBookmark>,
}

impl BorgAppState {
    pub fn new() -> Self {
        let mut state = Self::default();
        state.load_bookmarks();
        state.load_archive_bookmarks();
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
            self.session_passwords.insert(bookmark.path.clone(), pwd);
        }
        // Avoid duplicates by path
        if !self.bookmarks.iter().any(|b| b.path == bookmark.path) {
            self.bookmarks.push(bookmark);
            let _ = self.save_bookmarks();
        }
    }
}
