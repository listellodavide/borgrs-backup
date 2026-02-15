use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
pub use turso::{Builder, Connection, Database as TursoDatabase, Row, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoBookmark {
    pub id: Option<i64>,
    pub name: String,
    pub path: String,
    pub repo_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveBookmark {
    pub id: Option<i64>,
    pub repo_name: String,
    pub repo_path: String,
    pub compression: Option<String>,
    pub redundancy: Option<String>,
    pub use_custom_name: bool,
    pub archive_name: Option<String>,
    pub comment: Option<String>,
    pub tags: Option<String>,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: Option<i64>,
    pub task_name: String,
    pub repo_name: String,
    pub repo_path: String,
    pub archive_name: String,
    pub paths_to_backup: Vec<String>,
    pub compression: String,
    pub comment: Option<String>,
    pub tags: Option<Vec<String>>,
    pub schedule_type: String,
    pub weekday: i32,
    pub day_of_month: i32,
    pub hour: i32,
    pub minute: i32,
    pub repo_type: String,
    pub run_on_boot_if_missed: bool,
    pub execution_count: u32,
    pub active: bool,
    pub last_run: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobExecutionHistory {
    pub id: Option<i64>,
    pub job_name: String,
    pub start_time: String,
    pub end_time: Option<String>,
    pub status: String,
    pub error_message: Option<String>,
    pub files_processed: i64,
    pub bytes_processed: i64,
    pub archive_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobState {
    pub job_name: String,
    pub last_run: Option<String>,
    pub last_success: Option<String>,
    pub last_failure: Option<String>,
    pub consecutive_failures: i32,
    pub last_error: Option<String>,
    pub last_failure_alert: Option<String>,
    pub last_missed_alert: Option<String>,
}

pub struct Database {
    db: TursoDatabase,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database").finish()
    }
}

impl Database {
    pub async fn new(path: &Path) -> Result<Self> {
        let db = Builder::new_local(path.to_str().context("Invalid path")?)
            .build()
            .await?;
        let conn = db.connect()?;
        Self::init_schema(&conn).await?;
        Ok(Self { db })
    }

    async fn init_schema(conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS repo_bookmarks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                path TEXT NOT NULL,
                repo_type TEXT NOT NULL DEFAULT 'local'
            );",
            (),
        )
        .await?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS archive_bookmarks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_name TEXT NOT NULL,
                repo_path TEXT NOT NULL,
                compression TEXT,
                redundancy TEXT,
                use_custom_name BOOLEAN DEFAULT 0,
                archive_name TEXT,
                comment TEXT,
                tags TEXT,
                paths TEXT NOT NULL,
                UNIQUE(repo_path)
            );",
            (),
        )
        .await?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS scheduled_tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_name TEXT NOT NULL,
                repo_name TEXT NOT NULL,
                repo_path TEXT NOT NULL,
                archive_name TEXT NOT NULL,
                paths_to_backup TEXT NOT NULL,
                compression TEXT NOT NULL,
                comment TEXT,
                tags TEXT,
                schedule_type TEXT NOT NULL,
                weekday INTEGER DEFAULT 0,
                day_of_month INTEGER DEFAULT 0,
                hour INTEGER NOT NULL,
                minute INTEGER NOT NULL,
                run_on_boot_if_missed BOOLEAN DEFAULT 1,
                execution_count INTEGER DEFAULT 0,
                active BOOLEAN DEFAULT 1,
                last_run TEXT,
                repo_type TEXT NOT NULL DEFAULT 'local'
            );",
            (),
        )
        .await?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS job_execution_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                job_name TEXT NOT NULL,
                start_time TEXT NOT NULL,
                end_time TEXT,
                status TEXT NOT NULL,
                error_message TEXT,
                files_processed INTEGER DEFAULT 0,
                bytes_processed INTEGER DEFAULT 0,
                archive_name TEXT
            );",
            (),
        )
        .await?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS job_state (
                job_name TEXT PRIMARY KEY,
                last_run TEXT,
                last_success TEXT,
                last_failure TEXT,
                consecutive_failures INTEGER DEFAULT 0,
                last_error TEXT,
                last_failure_alert TEXT,
                last_missed_alert TEXT
            );",
            (),
        )
        .await?;

        Ok(())
    }

    pub async fn connect(&self) -> Result<Connection> {
        Ok(self.db.connect()?)
    }

    // --- Repo Bookmarks ---

    pub async fn add_repo(&self, repo: &RepoBookmark) -> Result<()> {
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO repo_bookmarks (name, path, repo_type) VALUES (?, ?, ?)",
            (
                repo.name.as_str(),
                repo.path.as_str(),
                repo.repo_type.as_str(),
            ),
        )
        .await?;
        Ok(())
    }

    pub async fn list_repos(&self) -> Result<Vec<RepoBookmark>> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query("SELECT id, name, path, repo_type FROM repo_bookmarks", ())
            .await?;
        let mut result = Vec::new();
        while let Some(row) = rows.next().await? {
            let id: i64 = row.get::<i64>(0)?;
            let name: String = row.get::<String>(1)?;
            let path: String = row.get::<String>(2)?;
            let repo_type: String = row.get::<String>(3)?;
            result.push(RepoBookmark {
                id: Some(id),
                name,
                path,
                repo_type,
            });
        }
        Ok(result)
    }

    pub async fn delete_repo(&self, name: &str) -> Result<()> {
        let conn = self.connect().await?;
        conn.execute("DELETE FROM repo_bookmarks WHERE name = ?", (name,))
            .await?;
        Ok(())
    }

    pub async fn repo_name_exists(&self, name: &str) -> Result<bool> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT count(*) FROM repo_bookmarks WHERE name = ?",
                (name,),
            )
            .await?;
        if let Some(row) = rows.next().await? {
            let count: i64 = row.get(0)?;
            Ok(count > 0)
        } else {
            Ok(false)
        }
    }

    // --- Archive Bookmarks ---

    pub async fn upsert_archive_bookmark(&self, bookmark: &ArchiveBookmark) -> Result<()> {
        let conn = self.connect().await?;
        let paths_json = serde_json::to_string(&bookmark.paths)?;
        conn.execute(
            "INSERT INTO archive_bookmarks (repo_name, repo_path, compression, redundancy, use_custom_name, archive_name, comment, tags, paths)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(repo_path) DO UPDATE SET
                repo_name=excluded.repo_name,
                compression=excluded.compression,
                redundancy=excluded.redundancy,
                use_custom_name=excluded.use_custom_name,
                archive_name=excluded.archive_name,
                comment=excluded.comment,
                tags=excluded.tags,
                paths=excluded.paths",
            (
                bookmark.repo_name.as_str(),
                bookmark.repo_path.as_str(),
                bookmark.compression.as_deref(),
                bookmark.redundancy.as_deref(),
                bookmark.use_custom_name,
                bookmark.archive_name.as_deref(),
                bookmark.comment.as_deref(),
                bookmark.tags.as_deref(),
                paths_json.as_str(),
            ),
        ).await?;
        Ok(())
    }

    pub async fn get_archive_bookmark(&self, repo_path: &str) -> Result<Option<ArchiveBookmark>> {
        let conn = self.connect().await?;
        let mut rows = conn.query(
            "SELECT id, repo_name, repo_path, compression, redundancy, use_custom_name, archive_name, comment, tags, paths FROM archive_bookmarks WHERE repo_path = ?",
            (repo_path,),
        ).await?;
        if let Some(row) = rows.next().await? {
            let id: i64 = row.get::<i64>(0)?;
            let repo_name: String = row.get::<String>(1)?;
            let repo_path: String = row.get::<String>(2)?;
            let compression: Option<String> = row.get::<Option<String>>(3)?;
            let redundancy: Option<String> = row.get::<Option<String>>(4)?;
            let use_custom_name: bool = row.get::<bool>(5)?;
            let archive_name: Option<String> = row.get::<Option<String>>(6)?;
            let comment: Option<String> = row.get::<Option<String>>(7)?;
            let tags: Option<String> = row.get::<Option<String>>(8)?;
            let paths_json: String = row.get::<String>(9)?;
            let paths: Vec<String> = serde_json::from_str(&paths_json)?;
            Ok(Some(ArchiveBookmark {
                id: Some(id),
                repo_name,
                repo_path,
                compression,
                redundancy,
                use_custom_name,
                archive_name,
                comment,
                tags,
                paths,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn list_archive_bookmarks(&self) -> Result<Vec<ArchiveBookmark>> {
        let conn = self.connect().await?;
        let mut rows = conn.query(
            "SELECT id, repo_name, repo_path, compression, redundancy, use_custom_name, archive_name, comment, tags, paths FROM archive_bookmarks",
            (),
        ).await?;
        let mut result = Vec::new();
        while let Some(row) = rows.next().await? {
            let id: i64 = row.get::<i64>(0)?;
            let repo_name: String = row.get::<String>(1)?;
            let repo_path: String = row.get::<String>(2)?;
            let compression: Option<String> = row.get::<Option<String>>(3)?;
            let redundancy: Option<String> = row.get::<Option<String>>(4)?;
            let use_custom_name: bool = row.get::<bool>(5)?;
            let archive_name: Option<String> = row.get::<Option<String>>(6)?;
            let comment: Option<String> = row.get::<Option<String>>(7)?;
            let tags: Option<String> = row.get::<Option<String>>(8)?;
            let paths_json: String = row.get::<String>(9)?;
            let paths: Vec<String> = serde_json::from_str(&paths_json)?;
            result.push(ArchiveBookmark {
                id: Some(id),
                repo_name,
                repo_path,
                compression,
                redundancy,
                use_custom_name,
                archive_name,
                comment,
                tags,
                paths,
            });
        }
        Ok(result)
    }

    // --- Scheduled Tasks ---

    pub async fn save_task(&self, task: &ScheduledTask) -> Result<()> {
        let conn = self.connect().await?;
        let paths_to_backup_str = task.paths_to_backup.join(",");
        let tags_str = task.tags.as_ref().map(|t| t.join(","));

        conn.execute(
            "INSERT OR REPLACE INTO scheduled_tasks (
                id, task_name, repo_name, repo_path, archive_name, paths_to_backup,
                compression, comment, tags, schedule_type, weekday, day_of_month,
                hour, minute, run_on_boot_if_missed, execution_count, active, last_run, repo_type
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                task.id.map(|i| i.into()).unwrap_or(Value::Null),
                task.task_name.clone().into(),
                task.repo_name.clone().into(),
                task.repo_path.clone().into(),
                task.archive_name.clone().into(),
                paths_to_backup_str.into(),
                task.compression.clone().into(),
                task.comment
                    .as_ref()
                    .map(|s| s.clone().into())
                    .unwrap_or(Value::Null),
                tags_str.map(|s| s.into()).unwrap_or(Value::Null),
                task.schedule_type.clone().into(),
                task.weekday.into(),
                task.day_of_month.into(),
                task.hour.into(),
                task.minute.into(),
                task.run_on_boot_if_missed.into(),
                (task.execution_count as i32).into(),
                task.active.into(),
                task.last_run
                    .as_ref()
                    .map(|s| s.clone().into())
                    .unwrap_or(Value::Null),
                task.repo_type.clone().into(),
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn list_tasks(&self) -> Result<Vec<ScheduledTask>> {
        let conn = self.connect().await?;
        let mut rows = conn.query(
            "SELECT id, task_name, repo_name, repo_path, archive_name, paths_to_backup, compression, comment, tags, schedule_type, weekday, day_of_month, hour, minute, run_on_boot_if_missed, execution_count, active, last_run, repo_type FROM scheduled_tasks",
            (),
        ).await?;
        let mut result = Vec::new();
        while let Some(row) = rows.next().await? {
            let id: i64 = row.get::<i64>(0)?;
            let task_name: String = row.get::<String>(1)?;
            let repo_name: String = row.get::<String>(2)?;
            let repo_path: String = row.get::<String>(3)?;
            let archive_name: String = row.get::<String>(4)?;
            let paths_json: String = row.get::<String>(5)?;
            // Manual CSV parsing as fallback if json fails, or just use CSV for now to match save_task
            let paths_to_backup: Vec<String> = if paths_json.starts_with('[') {
                serde_json::from_str(&paths_json)?
            } else {
                paths_json.split(',').map(|s| s.to_string()).collect()
            };
            let compression: String = row.get::<String>(6)?;
            let comment: Option<String> = row.get::<Option<String>>(7)?;
            let tags: Option<String> = row.get::<Option<String>>(8)?;
            let schedule_type: String = row.get::<String>(9)?;
            let weekday: i32 = row.get::<i32>(10)?;
            let day_of_month: i32 = row.get::<i32>(11)?;
            let hour: i32 = row.get::<i32>(12)?;
            let minute: i32 = row.get::<i32>(13)?;
            let run_on_boot_if_missed: bool = row.get::<bool>(14)?;
            let execution_count: i32 = row.get::<i32>(15)?;
            let active: bool = row.get::<bool>(16)?;
            let last_run: Option<String> = row.get::<Option<String>>(17)?;
            let repo_type: String = row.get::<String>(18)?;

            let tags_vec = tags.map(|s| s.split(',').map(|p| p.to_string()).collect());

            result.push(ScheduledTask {
                id: Some(id),
                task_name,
                repo_name,
                repo_path,
                archive_name,
                paths_to_backup,
                compression,
                comment,
                tags: tags_vec,
                schedule_type,
                weekday,
                day_of_month,
                hour,
                minute,
                run_on_boot_if_missed,
                execution_count: execution_count as u32,
                active,
                last_run,
                repo_type,
            });
        }
        Ok(result)
    }

    pub async fn delete_task(&self, id: i64) -> Result<()> {
        let conn = self.connect().await?;
        conn.execute("DELETE FROM scheduled_tasks WHERE id = ?", (id,))
            .await?;
        Ok(())
    }

    // --- Job Execution History ---

    pub async fn record_job_execution(&self, history: &JobExecutionHistory) -> Result<()> {
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO job_execution_history (job_name, start_time, end_time, status, error_message, files_processed, bytes_processed, archive_name)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            (
                history.job_name.as_str(),
                history.start_time.as_str(),
                history.end_time.as_deref(),
                history.status.as_str(),
                history.error_message.as_deref(),
                history.files_processed,
                history.bytes_processed,
                history.archive_name.as_deref(),
            ),
        ).await?;
        Ok(())
    }

    pub async fn list_history(&self, limit: i64) -> Result<Vec<JobExecutionHistory>> {
        let conn = self.connect().await?;
        let mut rows = conn.query(
            "SELECT id, job_name, start_time, end_time, status, error_message, files_processed, bytes_processed, archive_name FROM job_execution_history ORDER BY start_time DESC LIMIT ?",
            (limit,),
        ).await?;
        let mut result = Vec::new();
        while let Some(row) = rows.next().await? {
            let id: i64 = row.get::<i64>(0)?;
            let job_name: String = row.get::<String>(1)?;
            let start_time: String = row.get::<String>(2)?;
            let end_time: Option<String> = row.get::<Option<String>>(3)?;
            let status: String = row.get::<String>(4)?;
            let error_message: Option<String> = row.get::<Option<String>>(5)?;
            let files_processed: i64 = row.get::<i64>(6)?;
            let bytes_processed: i64 = row.get::<i64>(7)?;
            let archive_name: Option<String> = row.get::<Option<String>>(8)?;

            result.push(JobExecutionHistory {
                id: Some(id),
                job_name,
                start_time,
                end_time,
                status,
                error_message,
                files_processed,
                bytes_processed,
                archive_name,
            });
        }
        Ok(result)
    }

    // --- Job State ---

    pub async fn upsert_job_state(&self, state: &JobState) -> Result<()> {
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO job_state (job_name, last_run, last_success, last_failure, consecutive_failures, last_error, last_failure_alert, last_missed_alert)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(job_name) DO UPDATE SET
                last_run=excluded.last_run,
                last_success=excluded.last_success,
                last_failure=excluded.last_failure,
                consecutive_failures=excluded.consecutive_failures,
                last_error=excluded.last_error,
                last_failure_alert=excluded.last_failure_alert,
                last_missed_alert=excluded.last_missed_alert",
            (
                state.job_name.as_str(),
                state.last_run.as_deref(),
                state.last_success.as_deref(),
                state.last_failure.as_deref(),
                state.consecutive_failures,
                state.last_error.as_deref(),
                state.last_failure_alert.as_deref(),
                state.last_missed_alert.as_deref(),
            ),
        ).await?;
        Ok(())
    }

    pub async fn get_job_state(&self, job_name: &str) -> Result<Option<JobState>> {
        let conn = self.connect().await?;
        let mut rows = conn.query(
            "SELECT job_name, last_run, last_success, last_failure, consecutive_failures, last_error, last_failure_alert, last_missed_alert FROM job_state WHERE job_name = ?",
            (job_name,),
        ).await?;
        if let Some(row) = rows.next().await? {
            let job_name: String = row.get::<String>(0)?;
            let last_run: Option<String> = row.get::<Option<String>>(1)?;
            let last_success: Option<String> = row.get::<Option<String>>(2)?;
            let last_failure: Option<String> = row.get::<Option<String>>(3)?;
            let consecutive_failures: i32 = row.get::<i32>(4)?;
            let last_error: Option<String> = row.get::<Option<String>>(5)?;
            let last_failure_alert: Option<String> = row.get::<Option<String>>(6)?;
            let last_missed_alert: Option<String> = row.get::<Option<String>>(7)?;

            Ok(Some(JobState {
                job_name,
                last_run,
                last_success,
                last_failure,
                consecutive_failures,
                last_error,
                last_failure_alert,
                last_missed_alert,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn increment_task_execution(&self, id: i64, timestamp: String) -> Result<()> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE scheduled_tasks SET execution_count = execution_count + 1, last_run = ? WHERE id = ?",
            (timestamp.as_str(), id),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_repo_bookmarks_crud() -> Result<()> {
        let tmp_file = NamedTempFile::new()?;
        let db = Database::new(tmp_file.path()).await?;

        let repo = RepoBookmark {
            id: None,
            name: "test_repo".to_string(),
            path: "/tmp/test".to_string(),
            repo_type: "local".to_string(),
        };

        db.add_repo(&repo).await?;
        let repos = db.list_repos().await?;
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "test_repo");

        db.delete_repo("test_repo").await?;
        let repos = db.list_repos().await?;
        assert_eq!(repos.len(), 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_repo_name_exists() -> Result<()> {
        let tmp_file = NamedTempFile::new()?;
        let db = Database::new(tmp_file.path()).await?;

        let repo = RepoBookmark {
            id: None,
            name: "unique_repo".to_string(),
            path: "/tmp/unique".to_string(),
            repo_type: "local".to_string(),
        };

        assert!(!db.repo_name_exists("unique_repo").await?);
        db.add_repo(&repo).await?;
        assert!(db.repo_name_exists("unique_repo").await?);
        assert!(!db.repo_name_exists("other_repo").await?);

        Ok(())
    }

    #[tokio::test]
    async fn test_archive_bookmarks_upsert() -> Result<()> {
        let tmp_file = NamedTempFile::new()?;
        let db = Database::new(tmp_file.path()).await?;

        let bookmark = ArchiveBookmark {
            id: None,
            repo_name: "test_repo".to_string(),
            repo_path: "/tmp/repo".to_string(),
            compression: Some("zstd".to_string()),
            redundancy: None,
            use_custom_name: true,
            archive_name: Some("test_archive".to_string()),
            comment: Some("comment".to_string()),
            tags: Some("tag1,tag2".to_string()),
            paths: vec!["/path1".to_string(), "/path2".to_string()],
        };

        db.upsert_archive_bookmark(&bookmark).await?;
        let loaded = db.get_archive_bookmark("/tmp/repo").await?.unwrap();
        assert_eq!(loaded.archive_name.as_deref(), Some("test_archive"));
        assert_eq!(loaded.paths.len(), 2);

        // Update
        let mut updated = loaded.clone();
        updated.compression = Some("lz4".to_string());
        db.upsert_archive_bookmark(&updated).await?;
        let loaded = db.get_archive_bookmark("/tmp/repo").await?.unwrap();
        assert_eq!(loaded.compression.as_deref(), Some("lz4"));

        Ok(())
    }
}
