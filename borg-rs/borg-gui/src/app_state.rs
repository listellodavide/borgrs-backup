pub use borg_core::db::{ArchiveBookmark, Database, RepoBookmark, ScheduledTask};
pub use borg_core::scheduler::{BackupSchedule, ScheduleType};
use borg_core::credentials;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

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
        mut bookmark: RepoBookmark,
        password: Option<String>,
    ) -> anyhow::Result<()> {
        if let Some(db) = &self.db {
            // Generate UUID if not present (though it should be handled by caller usually)
            if bookmark.uuid.is_empty() {
                bookmark.uuid = Uuid::new_v4().to_string();
            }

            db.add_repo(&bookmark).await?;
            self.bookmarks = db.list_repos().await?;
            if let Some(pwd) = password {
                self.session_passwords
                    .insert(bookmark.path.clone(), pwd.clone());

                if let Ok(uuid) = Uuid::parse_str(&bookmark.uuid) {
                    let _ = credentials::store_password(&uuid, &pwd);
                }
            }
        }
        Ok(())
    }

    pub async fn delete_bookmark(&mut self, name: &str) -> anyhow::Result<()> {
        if let Some(db) = &self.db {
            // Find the bookmark to get its UUID before deleting
            let uuid_str = self.bookmarks.iter()
                .find(|b| b.name == name)
                .map(|b| b.uuid.clone());

            db.delete_repo(name).await?;
            self.bookmarks = db.list_repos().await?;

            if let Some(uuid_str) = uuid_str {
                if let Ok(uuid) = Uuid::parse_str(&uuid_str) {
                    let _ = credentials::delete_password(&uuid);
                }
            }
        }
        Ok(())
    }

    pub async fn add_bookmark_async(
        state: Arc<std::sync::Mutex<Self>>,
        mut bookmark: RepoBookmark,
        password: Option<String>,
    ) -> anyhow::Result<()> {
        let db = {
            let s = state.lock().unwrap();
            s.db.clone()
        };

        if let Some(db) = db {
            if bookmark.uuid.is_empty() {
                bookmark.uuid = Uuid::new_v4().to_string();
            }

            db.add_repo(&bookmark).await?;
            let repos = db.list_repos().await?;
            let mut s = state.lock().unwrap();
            s.bookmarks = repos;
            if let Some(pwd) = password {
                s.session_passwords
                    .insert(bookmark.path.clone(), pwd.clone());

                if let Ok(uuid) = Uuid::parse_str(&bookmark.uuid) {
                    let _ = credentials::store_password(&uuid, &pwd);
                }
            }
        }
        Ok(())
    }

    pub async fn delete_bookmark_async(
        state: Arc<std::sync::Mutex<Self>>,
        name: &str,
    ) -> anyhow::Result<()> {
        let (db, uuid_str) = {
            let s = state.lock().unwrap();
            let uuid = s.bookmarks.iter()
                .find(|b| b.name == name)
                .map(|b| b.uuid.clone());
            (s.db.clone(), uuid)
        };

        if let Some(db) = db {
            db.delete_repo(name).await?;
            let repos = db.list_repos().await?;
            let mut s = state.lock().unwrap();
            s.bookmarks = repos;

            if let Some(uuid_str) = uuid_str {
                if let Ok(uuid) = Uuid::parse_str(&uuid_str) {
                    let _ = credentials::delete_password(&uuid);
                }
            }
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
        // Find UUID for repo_name
        if let Some(bookmark) = self.bookmarks.iter().find(|b| b.name == repo_name) {
            if let Ok(uuid) = Uuid::parse_str(&bookmark.uuid) {
                credentials::store_password(&uuid, password)?;
                return Ok(());
            }
        }
        // Fallback for legacy or missing UUID (should not happen with new logic)
        // But we can't easily store without UUID in the new system.
        // For now, we'll error if UUID is missing.
        anyhow::bail!("Repository UUID not found for {}", repo_name)
    }

    pub fn get_password(&self, repo_name: &str) -> anyhow::Result<String> {
        if let Some(bookmark) = self.bookmarks.iter().find(|b| b.name == repo_name) {
            if let Ok(uuid) = Uuid::parse_str(&bookmark.uuid) {
                return credentials::get_password(&uuid);
            }
        }
        anyhow::bail!("Repository UUID not found for {}", repo_name)
    }
}
