pub use borg_core::db::{ArchiveBookmark, Database, RepoBookmark, ScheduledTask};
pub use borg_core::scheduler::{BackupSchedule, ScheduleType};
use keyring::Entry;
use std::collections::HashMap;
use std::sync::Arc;

const KEYRING_SERVICE: &str = "borg-gui";

#[derive(Debug, Default)]
pub struct BorgAppState {
    pub current_repo: Option<String>,
    pub is_processing: bool,
    pub bookmarks: Vec<RepoBookmark>,
    pub session_passwords: HashMap<String, String>, // repo_path -> password
    pub archive_bookmarks: Vec<ArchiveBookmark>,
    pub scheduled_tasks: Vec<ScheduledTask>,

    pub archive_cache: Option<(String, Vec<crate::commands::FileEntry>)>,
    pub db: Option<Arc<Database>>,
    pub auth_oneshot: Option<tokio::sync::oneshot::Sender<String>>,
    pub auth_retry_count: u32,
}

impl BorgAppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_db(&mut self, db: Arc<Database>) {
        self.db = Some(db);
    }

    pub async fn load_all(&mut self) -> anyhow::Result<()> {
        if let Some(db) = &self.db {
            self.bookmarks = db.list_repos().await?;
            self.archive_bookmarks = db.list_archive_bookmarks().await?;
            self.scheduled_tasks = db.list_tasks().await?;
        }
        Ok(())
    }

    pub async fn add_bookmark(
        &mut self,
        bookmark: RepoBookmark,
        password: Option<String>,
    ) -> anyhow::Result<()> {
        if let Some(db) = &self.db {
            db.add_repo(&bookmark).await?;
            self.bookmarks = db.list_repos().await?;
            if let Some(pwd) = password {
                self.session_passwords
                    .insert(bookmark.path.clone(), pwd.clone());
                let _ = self.store_password(&bookmark.name, &pwd);
            }
        }
        Ok(())
    }

    pub async fn delete_bookmark(&mut self, name: &str) -> anyhow::Result<()> {
        if let Some(db) = &self.db {
            db.delete_repo(name).await?;
            self.bookmarks = db.list_repos().await?;
        }
        Ok(())
    }

    pub async fn add_bookmark_async(
        state: Arc<std::sync::Mutex<Self>>,
        bookmark: RepoBookmark,
        password: Option<String>,
    ) -> anyhow::Result<()> {
        let db = {
            let s = state.lock().unwrap();
            s.db.clone()
        };

        if let Some(db) = db {
            db.add_repo(&bookmark).await?;
            let repos = db.list_repos().await?;
            let mut s = state.lock().unwrap();
            s.bookmarks = repos;
            if let Some(pwd) = password {
                s.session_passwords
                    .insert(bookmark.path.clone(), pwd.clone());
                let _ = s.store_password(&bookmark.name, &pwd);
            }
        }
        Ok(())
    }

    pub async fn delete_bookmark_async(
        state: Arc<std::sync::Mutex<Self>>,
        name: &str,
    ) -> anyhow::Result<()> {
        let db = {
            let s = state.lock().unwrap();
            s.db.clone()
        };
        if let Some(db) = db {
            db.delete_repo(name).await?;
            let repos = db.list_repos().await?;
            let mut s = state.lock().unwrap();
            s.bookmarks = repos;
        }
        Ok(())
    }

    pub async fn upsert_archive_bookmark_async(
        state: Arc<std::sync::Mutex<Self>>,
        bookmark: ArchiveBookmark,
    ) -> anyhow::Result<()> {
        let db = {
            let s = state.lock().unwrap();
            s.db.clone()
        };
        if let Some(db) = db {
            db.upsert_archive_bookmark(&bookmark).await?;
            let bookmarks = db.list_archive_bookmarks().await?;
            let mut s = state.lock().unwrap();
            s.archive_bookmarks = bookmarks;
        }
        Ok(())
    }

    pub async fn save_task_async(
        state: Arc<std::sync::Mutex<Self>>,
        task: ScheduledTask,
    ) -> anyhow::Result<()> {
        let db = {
            let s = state.lock().unwrap();
            s.db.clone()
        };
        if let Some(db) = db {
            db.save_task(&task).await?;
            let tasks = db.list_tasks().await?;
            let mut s = state.lock().unwrap();
            s.scheduled_tasks = tasks;
        }
        Ok(())
    }

    pub async fn delete_task_async(
        state: Arc<std::sync::Mutex<Self>>,
        id: i64,
    ) -> anyhow::Result<()> {
        let db = {
            let s = state.lock().unwrap();
            s.db.clone()
        };
        if let Some(db) = db {
            db.delete_task(id).await?;
            let tasks = db.list_tasks().await?;
            let mut s = state.lock().unwrap();
            s.scheduled_tasks = tasks;
        }
        Ok(())
    }

    pub async fn upsert_archive_bookmark(
        &mut self,
        bookmark: ArchiveBookmark,
    ) -> anyhow::Result<()> {
        if let Some(db) = &self.db {
            db.upsert_archive_bookmark(&bookmark).await?;
            self.archive_bookmarks = db.list_archive_bookmarks().await?;
        }
        Ok(())
    }

    pub fn get_archive_bookmark_for_repo(&self, repo_path: &str) -> Option<ArchiveBookmark> {
        self.archive_bookmarks
            .iter()
            .find(|b| b.repo_path == repo_path)
            .cloned()
    }

    pub async fn save_task(&mut self, task: ScheduledTask) -> anyhow::Result<()> {
        if let Some(db) = &self.db {
            db.save_task(&task).await?;
            self.scheduled_tasks = db.list_tasks().await?;
        }
        Ok(())
    }

    pub async fn delete_task(&mut self, id: i64) -> anyhow::Result<()> {
        if let Some(db) = &self.db {
            db.delete_task(id).await?;
            self.scheduled_tasks = db.list_tasks().await?;
        }
        Ok(())
    }

    pub fn store_password(&self, repo_name: &str, password: &str) -> anyhow::Result<()> {
        let account = format!("borgrs_{}", repo_name);
        let entry = Entry::new(KEYRING_SERVICE, &account)?;
        entry.set_password(password)?;
        Ok(())
    }

    pub fn get_password(&self, repo_name: &str) -> anyhow::Result<String> {
        let account = format!("borgrs_{}", repo_name);
        let entry = Entry::new(KEYRING_SERVICE, &account)?;
        let password = entry.get_password()?;
        Ok(password)
    }
}
