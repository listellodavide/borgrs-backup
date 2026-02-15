use crate::app_state::BorgAppState as RustAppState;
use crate::app_state::RepoBookmark;
use crate::commands;
use crate::commands::{BackupProgress, RestoreProgress};
use crate::{ArchiveContentLogic, ArchiveEntry, ArchiveFilesLogic, SchedulerLogic};
use slint::{ComponentHandle, Model, SharedString};
use std::path::Path;
use super::scheduler_bridge;

struct GuiBackupProgress {
    window_weak: slint::Weak<MainWindow>,
}

impl BackupProgress for GuiBackupProgress {
    fn on_file_start(&self, path: &Path) {
        let path_str = path.to_string_lossy().to_string();
        let window_weak = self.window_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = window_weak.upgrade() {
                let dashboard = window.global::<DashboardLogic>();
                let current_text = dashboard.get_terminal_text();
                dashboard.set_terminal_text(format!("{}\nProcessing: {}", current_text, path_str).into());
            }
        });
    }

    fn on_file_complete(&self, _path: &Path, _size: u64, _chunks: usize) {}

    fn on_file_skipped(&self, path: &Path, reason: &str) {
        let path_str = path.to_string_lossy().to_string();
        let reason = reason.to_string();
        let window_weak = self.window_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = window_weak.upgrade() {
                let dashboard = window.global::<DashboardLogic>();
                let current_text = dashboard.get_terminal_text();
                dashboard.set_terminal_text(format!("{}\nSkipped: {} ({})", current_text, path_str, reason).into());
            }
        });
    }

    fn on_progress(&self, processed: u64, total: u64, filename: Option<&str>) {
        let progress = if total > 0 {
            processed as f32 / total as f32
        } else {
            1.0
        };
        let window_weak = self.window_weak.clone();
        let file_opt = filename.map(|s| s.to_string());
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = window_weak.upgrade() {
                window.global::<AppState>().set_progress(progress);
                if let Some(file) = file_opt {
                    let dash = window.global::<DashboardLogic>();
                    let mut files: Vec<slint::SharedString> = dash.get_processed_files().iter().cloned().collect();
                    files.insert(0, file.into());
                    if files.len() > 50 {
                        files.truncate(50);
                    }
                    dash.set_processed_files(slint::VecModel::from(files).into());
                }
            }
        });
    }

    fn on_error(&self, path: &Path, error: &str) {
        let path_str = path.to_string_lossy().to_string();
        let error = error.to_string();
        let window_weak = self.window_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = window_weak.upgrade() {
                let dashboard = window.global::<DashboardLogic>();
                let current_text = dashboard.get_terminal_text();
                dashboard.set_terminal_text(format!("{}\nError at {}: {}", current_text, path_str, error).into());
            }
        });
    }
}

// Restore progress reporter that updates UI
struct GuiRestoreProgress {
    window_weak: slint::Weak<MainWindow>,
    totals: std::sync::Arc<std::sync::Mutex<(u64, u64)>>,
    file_sizes: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, u64>>>,
}

impl RestoreProgress for GuiRestoreProgress {
    fn on_start(&self, _total_files: u64, total_bytes: u64) {
        {
            let mut t = self.totals.lock().unwrap();
            t.0 = 0;
            t.1 = total_bytes;
        }
        let window_weak = self.window_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = window_weak.upgrade() {
                window.global::<AppState>().set_progress(0.0);
            }
        });
    }

    fn on_file_start(&self, path: &Path, size: u64) {
        let path_str = path.to_string_lossy().to_string();
        {
            let mut m = self.file_sizes.lock().unwrap();
            m.insert(path_str.clone(), size);
        }
        let window_weak = self.window_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = window_weak.upgrade() {
                let dash = window.global::<DashboardLogic>();
                let current = dash.get_terminal_text();
                dash.set_terminal_text(format!("{}\nRestoring: {}", current, path_str).into());
            }
        });
    }

    fn on_file_complete(&self, path: &Path) {
        let key = path.to_string_lossy().to_string();
        let added = {
            let mut m = self.file_sizes.lock().unwrap();
            m.remove(&key).unwrap_or(0)
        };
        let (processed, total) = {
            let mut t = self.totals.lock().unwrap();
            t.0 = t.0.saturating_add(added);
            (t.0, t.1)
        };
        let window_weak = self.window_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = window_weak.upgrade() {
                let p = if total > 0 { processed as f32 / total as f32 } else { 1.0 };
                window.global::<AppState>().set_progress(p);
            }
        });
    }

    fn on_error(&self, path: &Path, error: &str) {
        let p = path.to_string_lossy().to_string();
        let e = error.to_string();
        let window_weak = self.window_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = window_weak.upgrade() {
                let dash = window.global::<DashboardLogic>();
                let current = dash.get_terminal_text();
                dash.set_terminal_text(format!("{}\nError restoring {}: {}", current, p, e).into());
            }
        });
    }

    fn on_finish(&self) {
        let window_weak = self.window_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = window_weak.upgrade() {
                window.global::<AppState>().set_progress(1.0);
            }
        });
    }
}

use std::sync::{Arc, Mutex};
use super::{MainWindow, AppState, DashboardLogic, InitWizardLogic, RestoreLogic, RepoItem, NewArchiveWizardLogic};

pub fn init_bridge(window: &MainWindow, state: Arc<Mutex<RustAppState>>) {
    let window_weak = window.as_weak();
    let state_clone = state.clone();

    // AppState Navigation
    let app_state = window.global::<AppState>();
    app_state.on_navigate({
        let window_weak = window_weak.clone();
        move |view| {
            if let Some(window) = window_weak.upgrade() {
                window.global::<AppState>().set_current_view(view);
            }
        }
    });

    // Dashboard Logic
    let dashboard = window.global::<DashboardLogic>();
    
    // Password Dialog Handlers
    app_state.on_password_dialog_cancelled({
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                window.global::<AppState>().set_show_password_dialog(false);
                window.global::<AppState>().set_is_processing(false);
            }
        }
    });

    app_state.on_password_dialog_submitted({
        let window_weak = window_weak.clone();
        let state_clone = state.clone();
        move |password: SharedString| {
            if let Some(window) = window_weak.upgrade() {
                window.global::<AppState>().set_show_password_dialog(false);
                
                let dashboard = window.global::<DashboardLogic>();
                let active_index = dashboard.get_active_repo_index();
                
                let repo_path = {
                    let app = window.global::<AppState>();
                    let pending_path = app.get_pending_auth_repo_path().to_string();
                    if !pending_path.is_empty() {
                        // find repo name for this path
                        let s = state_clone.lock().unwrap();
                        s.bookmarks.iter().find(|b| b.path == pending_path).map(|b| b.name.clone()).unwrap_or_default()
                    } else {
                        let dashboard = window.global::<DashboardLogic>();
                        let active_index = dashboard.get_active_repo_index();
                        let s = state_clone.lock().unwrap();
                        if let Some(bm) = s.bookmarks.get(active_index as usize) {
                            bm.name.clone()
                        } else {
                            return;
                        }
                    }
                };

                let repo_path = {
                    let app = window.global::<AppState>();
                    let pending_path = app.get_pending_auth_repo_path().to_string();
                    if !pending_path.is_empty() {
                        pending_path
                    } else {
                        let dashboard = window.global::<DashboardLogic>();
                        let active_index = dashboard.get_active_repo_index();
                        let s = state_clone.lock().unwrap();
                        if let Some(bm) = s.bookmarks.get(active_index as usize) {
                            bm.path.clone()
                        } else {
                            return;
                        }
                    }
                };

                // Store in session
                {
                    let mut s = state_clone.lock().unwrap();
                    s.session_passwords.insert(repo_path.clone(), password.to_string());
                    // Also store in keyring
                    let repo_name = {
                        s.bookmarks.iter().find(|b| b.path == repo_path).map(|b| b.name.clone()).unwrap_or_else(|| "".to_string())
                    };
                    if !repo_name.is_empty() {
                        let _ = s.store_password(&repo_name, &password);
                    }
                }

                // Complete interactive auth if pending
                let auth_oneshot = {
                    let mut s = state_clone.lock().unwrap();
                    s.auth_oneshot.take()
                };

                if let Some(tx) = auth_oneshot {
                    let _ = tx.send(password.to_string());
                    return;
                }

                // Retry last action based on pending_auth_action
                let action = window.global::<AppState>().get_pending_auth_action();
                let _ = slint::invoke_from_event_loop({
                    let window_weak = window_weak.clone();
                    move || {
                        if let Some(w) = window_weak.upgrade() {
                            let app = w.global::<AppState>();
                            if action.as_str() == "restore" {
                                w.global::<RestoreLogic>().invoke_start_restore();
                            } else if action.as_str() == "save_scheduled_task" {
                                w.global::<SchedulerLogic>().invoke_save_task();
                            } else {
                                w.global::<DashboardLogic>().invoke_backup_clicked();
                            }
                            app.set_pending_auth_action(SharedString::from(""));
                            app.set_pending_auth_repo_path(SharedString::from(""));
                        }
                    }
                });
            }
        }
    });
    
    dashboard.on_new_archive_clicked({
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                window.global::<AppState>().set_current_view(SharedString::from("new_archive"));
                let wizard = window.global::<NewArchiveWizardLogic>();
                wizard.set_current_step(0);
                wizard.set_backup_paths(std::rc::Rc::new(slint::VecModel::default()).into());
            }
        }
    });

    dashboard.on_backup_clicked({
        let window_weak = window_weak.clone();
        let state_clone = state.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                let dashboard = window.global::<DashboardLogic>();
                let active_index = dashboard.get_active_repo_index();

                let (repo_path, repo_password) = {
                    let s = state_clone.lock().unwrap();
                    if let Some(bm) = s.bookmarks.get(active_index as usize) {
                        let pwd = s.session_passwords.get(&bm.path).cloned();
                        (bm.path.clone(), pwd)
                    } else {
                        return;
                    }
                };

                // Load archive settings for this repo
                let bookmark = {
                    let s = state_clone.lock().unwrap();
                    s.get_archive_bookmark_for_repo(&repo_path)
                };

                if bookmark.is_none() {
                    dashboard.set_terminal_text("No archive configuration found for this repository. Use +NEW ARCHIVE first.".into());
                    return;
                }
                let bm = bookmark.unwrap();

                // Build tags vec
                let tags_vec: Option<Vec<String>> = bm.tags.as_ref().and_then(|s| {
                    let t: Vec<String> = s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
                    if t.is_empty() { None } else { Some(t) }
                });

                // Determine archive name
                let name = if bm.use_custom_name {
                    bm.archive_name.clone().unwrap_or_else(|| crate::commands::generate_managed_archive_name())
                } else {
                    crate::commands::generate_managed_archive_name()
                };

                window.global::<AppState>().set_is_processing(true);
                window.global::<AppState>().set_progress(0.05);
                dashboard.set_terminal_text(format!("$ borg create -r {} {}\nStarting backup of {} paths...", repo_path, name, bm.paths.len()).into());

                let window_weak2 = window_weak.clone();
                let repo_path_clone = repo_path.clone();
                let progress_reporter = GuiBackupProgress { window_weak: window_weak.clone() };
                
                // Clone tags_vec before moving it into the task
                let tags_vec_for_main_task = tags_vec.clone();

                tokio::spawn(async move {
                    let res = crate::commands::create_archive(
                        &repo_path_clone,
                        repo_password.as_deref(),
                        &name,
                        bm.paths.clone(),
                        bm.compression.as_deref().unwrap_or("zstd,3"),
                        bm.comment.clone(),
                        tags_vec_for_main_task,
                        Some(Box::new(progress_reporter)),
                    ).await;

                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = window_weak2.upgrade() {
                            match res {
                                Ok(summary) => {
                                    w.global::<AppState>().set_is_processing(false);
                                    w.global::<AppState>().set_progress(1.0);

                                    // Build new ArchiveEntry and prepend to archives model
                                    let dashboard = w.global::<DashboardLogic>();
                                    use slint::{Model, VecModel};
                                    // Extract current model to Vec
                                    let current_model = dashboard.get_archives();
                                    let mut list: Vec<ArchiveEntry> = Vec::new();
                                    let count = current_model.row_count();
                                    for i in 0..count {
                                        if let Some(item) = current_model.row_data(i) {
                                            list.push(item);
                                        }
                                    }
                                    // Prepend new entry with hostname/comment/tags
                                    let new_entry = ArchiveEntry {
                                        name: summary.name.into(),
                                        date: summary.date.into(),
                                        size: summary.size.into(),
                                        hostname: summary.hostname.into(),
                                        comment: summary.comment.into(),
                                        tags: summary.tags.into(),
                                    };
                                    list.insert(0, new_entry);
                                    let model = std::rc::Rc::new(VecModel::from(list));
                                    dashboard.set_archives(model.into());

                                    dashboard.set_terminal_text(format!("Backup '{}' completed successfully.", name).into());
                                }
                                Err(e) => {
                                    let err_msg = e.to_string();
                                    if err_msg.contains("Repository is locked") {
                                        // Extract lock information
                                        let lock_task = if let Some(start) = err_msg.find("task '") {
                                            if let Some(end) = err_msg[start+6..].find("'") {
                                                &err_msg[start+6..start+6+end]
                                            } else {
                                                "unknown"
                                            }
                                        } else {
                                            "unknown"
                                        };

                                        w.global::<AppState>().set_is_processing(false);
                                        w.global::<DashboardLogic>().set_terminal_text(
                                            format!(
                                                "⏳ Backup queued: Waiting for current backup (task: {}) to complete...\n\nYour backup will start automatically once the repository is unlocked.",
                                                lock_task
                                            ).into()
                                        );

                                        // Queue the task for retry (every 5 seconds, up to 30 minutes)
                                        let window_weak_retry = window_weak2.clone();
                                        let repo_path_retry = repo_path_clone.clone();
                                        let archive_name_retry = name.clone();
                                        let repo_password_retry = repo_password.clone();
                                        let bm_retry = bm.clone();
                                        let tags_vec_retry = tags_vec.clone();

                                        tokio::spawn(async move {
                                            let mut retry_count = 0;
                                            let max_retries = 360; // 30 minutes / 5 seconds
                                            let should_exit = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

                                            while !should_exit.load(std::sync::atomic::Ordering::Relaxed) && retry_count < max_retries {
                                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                                                retry_count += 1;

                                                if retry_count > max_retries {
                                                    let _ = slint::invoke_from_event_loop(move || {
                                                        if let Some(w) = window_weak_retry.upgrade() {
                                                            w.global::<DashboardLogic>().set_terminal_text(
                                                                "❌ Queued backup timeout: Repository remained locked for 30 minutes.".into()
                                                            );
                                                            w.global::<AppState>().set_is_processing(false);
                                                        }
                                                    });
                                                    break;
                                                }

                                                // Try again
                                                let progress_reporter = GuiBackupProgress { window_weak: window_weak_retry.clone() };
                                                let res = crate::commands::create_archive(
                                                    &repo_path_retry,
                                                    repo_password_retry.as_deref(),
                                                    &archive_name_retry,
                                                    bm_retry.paths.clone(),
                                                    bm_retry.compression.as_deref().unwrap_or("zstd,3"),
                                                    bm_retry.comment.clone(),
                                                    tags_vec_retry.clone(),
                                                    Some(Box::new(progress_reporter)),
                                                ).await;

                                                let should_exit_clone = should_exit.clone();
                                                let _ = slint::invoke_from_event_loop({
                                                    let window_weak_notify = window_weak_retry.clone();
                                                    let archive_name_notify = archive_name_retry.clone();
                                                    move || {
                                                        if let Some(w) = window_weak_notify.upgrade() {
                                                            match res {
                                                                Ok(summary) => {
                                                                    w.global::<AppState>().set_is_processing(false);
                                                                    w.global::<AppState>().set_progress(1.0);

                                                                    // Update archives list
                                                                    let dashboard = w.global::<DashboardLogic>();
                                                                    use slint::{Model, VecModel};
                                                                    let current_model = dashboard.get_archives();
                                                                    let mut list: Vec<ArchiveEntry> = Vec::new();
                                                                    let count = current_model.row_count();
                                                                    for i in 0..count {
                                                                        if let Some(item) = current_model.row_data(i) {
                                                                            list.push(item);
                                                                        }
                                                                    }
                                                                    let new_entry = ArchiveEntry {
                                                                        name: summary.name.into(),
                                                                        date: summary.date.into(),
                                                                        size: summary.size.into(),
                                                                        hostname: summary.hostname.into(),
                                                                        comment: summary.comment.into(),
                                                                        tags: summary.tags.into(),
                                                                    };
                                                                    list.insert(0, new_entry);
                                                                    let model = std::rc::Rc::new(VecModel::from(list));
                                                                    dashboard.set_archives(model.into());

                                                                    dashboard.set_terminal_text(
                                                                        format!("✅ Queued backup '{}' completed successfully.", archive_name_notify).into()
                                                                    );
                                                                    should_exit_clone.store(true, std::sync::atomic::Ordering::Relaxed);
                                                                }
                                                                Err(err) => {
                                                                    if !err.to_string().contains("Repository is locked") {
                                                                        w.global::<AppState>().set_is_processing(false);
                                                                        w.global::<DashboardLogic>().set_terminal_text(
                                                                            format!("❌ Queued backup failed: {}", err).into()
                                                                        );
                                                                        should_exit_clone.store(true, std::sync::atomic::Ordering::Relaxed);
                                                                    }
                                                                    // If still locked, loop continues
                                                                }
                                                            }
                                                        }
                                                    }
                                                });
                                            }
                                        });
                                    } else if err_msg.contains("Invalid passphrase") || err_msg.contains("Passphrase required") {
                                        w.global::<AppState>().set_pending_auth_action(SharedString::from("backup"));
                                        w.global::<AppState>().set_show_password_dialog(true);
                                        w.global::<AppState>().set_password_dialog_message(format!("Authentication failed for {}. Please enter passphrase:", repo_path_clone).into());
                                        w.global::<DashboardLogic>().set_terminal_text(format!("Authentication failed: {}", e).into());
                                    } else {
                                        w.global::<AppState>().set_is_processing(false);
                                        w.global::<AppState>().set_progress(0.0);
                                        w.global::<DashboardLogic>().set_terminal_text(format!("❌ Backup failed: {}", e).into());
                                    }
                                }
                            }
                        }
                    });
                });
            }
        }
    });

    dashboard.on_refresh_archives(move || {
        println!("Refreshing archives...");
    });

    dashboard.on_select_repo({
        let window_weak = window_weak.clone();
        let state_clone = state.clone();
        move |index| {
            if let Some(window) = window_weak.upgrade() {
                let dashboard = window.global::<DashboardLogic>();

                // Handle section header click (index -1)
                if index == -1 {
                    dashboard.set_active_repo_index(-1);
                    dashboard.set_has_selected_repo(false);
                    dashboard.set_is_scheduled_tasks_active(false);
                    window.global::<AppState>().set_current_view("dashboard".into());
                    return;
                }

                dashboard.set_active_repo_index(index);
                dashboard.set_has_selected_repo(true);
                dashboard.set_is_scheduled_tasks_active(false);
                window.global::<AppState>().set_current_view("dashboard".into());

                let (repo_path, bookmarks, password) = {
                    let s = state_clone.lock().unwrap();
                    let path = s.bookmarks.get(index as usize).map(|bm| bm.path.clone());
                    let pwd = path.as_ref().and_then(|p| s.session_passwords.get(p).cloned());
                    (path, s.bookmarks.clone(), pwd)
                };

                if let Some(path) = repo_path {
                    // Update pending archive state for this repo
                    let (bookmark, repo_name) = {
                        let s = state_clone.lock().unwrap();
                        let bm = s.get_archive_bookmark_for_repo(&path);
                        let name = s.bookmarks.get(index as usize).map(|b| b.name.clone());
                        (bm, name)
                    };
                    
                    if let Some(bm) = bookmark {
                        dashboard.set_has_pending_archive(true);
                        let paths: Vec<SharedString> = bm.paths.into_iter().map(SharedString::from).collect();
                        dashboard.set_pending_archive_paths(std::rc::Rc::new(slint::VecModel::from(paths)).into());
                    } else {
                        dashboard.set_has_pending_archive(false);
                    }

                    // Try to get password from keyring if not in session
                    let password_for_check = if password.is_none() {
                        if let Some(name) = repo_name {
                            let s = state_clone.lock().unwrap();
                            if let Ok(pwd) = s.get_password(&name) {
                                // Update session cache
                                drop(s);
                                let mut s = state_clone.lock().unwrap();
                                s.session_passwords.insert(path.clone(), pwd.clone());
                                Some(pwd)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        password
                    };

                    // Load archives for the selected repository
                    let window_weak2 = window_weak.clone();
                    let repo_path_clone = path.clone();
                    tokio::spawn(async move {
                        match async {
                            let storage = borg_core::storage::StorageConfig::Local {
                                path: std::path::PathBuf::from(&repo_path_clone)
                            };
                            let op = borg_core::storage::build_operator(storage)
                                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                            let repo = borg_core::repository::Repository::open(
                                op,
                                repo_path_clone.clone(),
                                password_for_check.as_deref()
                            ).await
                                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                            let manifest = repo.load_manifest()
                                .await
                                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                            Ok::<_, anyhow::Error>(manifest.archives)
                        }.await {
                            Ok(archive_list) => {
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(w) = window_weak2.upgrade() {
                                        let archive_entries: Vec<ArchiveEntry> = archive_list
                                            .into_iter()
                                            .map(|archive| {
                                                ArchiveEntry {
                                                    name: archive.name.clone().into(),
                                                    date: archive.time.to_string().into(),
                                                    size: "".into(),
                                                    hostname: archive.hostname.clone().into(),
                                                    comment: archive.comment.unwrap_or_default().into(),
                                                    tags: archive.tags.unwrap_or_default().join(", ").into(),
                                                }
                                            })
                                            .collect();
                                        let archives_model = std::rc::Rc::new(slint::VecModel::from(archive_entries));
                                        w.global::<DashboardLogic>().set_archives(archives_model.into());
                                    }
                                });
                            }
                            Err(e) => {
                                let error_msg = format!("Failed to load archives: {}", e);
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(w) = window_weak2.upgrade() {
                                        w.global::<DashboardLogic>()
                                            .set_terminal_text(error_msg.into());
                                    }
                                });
                            }
                        }
                    });
                }
                
                if let Some(bookmark) = bookmarks.get(index as usize) {
                    println!("Selected repo: {} at {}", bookmark.name, bookmark.path);
                }
            }
        }
    });

    dashboard.on_select_scheduled_tasks({
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                let dashboard = window.global::<DashboardLogic>();
                dashboard.set_is_scheduled_tasks_active(true);
                dashboard.set_has_selected_repo(false);
                window.global::<AppState>().set_current_view("scheduled_tasks".into());

                // Refresh the repository list in the scheduler
                let scheduler = window.global::<SchedulerLogic>();
                scheduler.invoke_refresh_repositories();
            }
        }
    });

    dashboard.on_open_restore({
        let window_weak = window_weak.clone();
        move |entry| {
            if let Some(window) = window_weak.upgrade() {
                window.global::<AppState>().set_current_view(SharedString::from("restore"));
                let restore = window.global::<RestoreLogic>();
                restore.set_selected_archive(entry.name);
                restore.set_use_original_paths(true);
                restore.set_restore_path(SharedString::from(""));
            }
        }
    });

    // New Archive Wizard Logic
    let new_archive = window.global::<NewArchiveWizardLogic>();

    new_archive.on_back({
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                let wizard = window.global::<NewArchiveWizardLogic>();
                let current = wizard.get_current_step();
                if current > 0 {
                    wizard.set_current_step(current - 1);
                }
            }
        }
    });

    new_archive.on_next({
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                let wizard = window.global::<NewArchiveWizardLogic>();
                wizard.set_current_step(wizard.get_current_step() + 1);
            }
        }
    });

    new_archive.on_cancel({
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                window.global::<AppState>().set_current_view(SharedString::from("dashboard"));
            }
        }
    });

    new_archive.on_add_path({
        let window_weak = window_weak.clone();
        move || {
            // Open native file/folder picker using rfd in a blocking task
            let window_weak2 = window_weak.clone();
            tokio::task::spawn_blocking(move || {
                // Try pick a folder first; if none, try pick a single file
                let picked = rfd::FileDialog::new().pick_folder().or_else(|| rfd::FileDialog::new().pick_file());
                if let Some(pathbuf) = picked {
                    let path_str = pathbuf.to_string_lossy().to_string();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window_weak2.upgrade() {
                            let wizard = window.global::<NewArchiveWizardLogic>();
                            let mut paths: Vec<SharedString> = wizard.get_backup_paths().iter().collect();
                            if !paths.iter().any(|p| p.as_str() == path_str) {
                                paths.push(SharedString::from(path_str));
                            }
                            let model = std::rc::Rc::new(slint::VecModel::from(paths));
                            wizard.set_backup_paths(model.into());
                        }
                    });
                }
            });
        }
    });

    new_archive.on_remove_path({
        let window_weak = window_weak.clone();
        move |index| {
            if let Some(window) = window_weak.upgrade() {
                let wizard = window.global::<NewArchiveWizardLogic>();
                let mut paths: Vec<SharedString> = wizard.get_backup_paths().iter().collect();
                if (index as usize) < paths.len() {
                    paths.remove(index as usize);
                }
                let model = std::rc::Rc::new(slint::VecModel::from(paths));
                wizard.set_backup_paths(model.into());
            }
        }
    });

    new_archive.on_finish({
        let window_weak = window_weak.clone();
        let state_clone = state.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                let wizard = window.global::<NewArchiveWizardLogic>();
                let dashboard_logic = window.global::<DashboardLogic>();
                let active_index = dashboard_logic.get_active_repo_index();
                
                let (repo_name, repo_path, _repo_password) = {
                    let s = state_clone.lock().unwrap();
                    if let Some(bookmark) = s.bookmarks.get(active_index as usize) {
                        let pwd = s.session_passwords.get(&bookmark.path).cloned();
                        (bookmark.name.clone(), bookmark.path.clone(), pwd)
                    } else {
                        return;
                    }
                };

                // Persist archive bookmark for this repo
                let paths_strings: Vec<String> = wizard.get_backup_paths().iter().map(|s| s.to_string()).collect();
                let compression = wizard.get_compression_level().to_string();
                let redundancy = wizard.get_redundancy().to_string();
                let use_custom_name = wizard.get_use_custom_name();
                let archive_name_opt = if use_custom_name { Some(wizard.get_archive_name().to_string()) } else { None };
                let comment_opt = {
                    let s = wizard.get_comment().to_string();
                    if s.is_empty() { None } else { Some(s) }
                };
                let tags_opt = {
                    let s = wizard.get_tags().to_string();
                    if s.is_empty() { None } else { Some(s) }
                };
                let window_weak2 = window_weak.clone();
                let state_arc = state_clone.clone();
                tokio::spawn(async move {
                    let res = crate::app_state::BorgAppState::upsert_archive_bookmark_async(state_arc, crate::app_state::ArchiveBookmark {
                        id: None,
                        repo_name,
                        repo_path: repo_path.clone(),
                        compression: Some(compression.clone()),
                        redundancy: Some(redundancy),
                        use_custom_name,
                        archive_name: archive_name_opt.clone(),
                        comment: comment_opt.clone(),
                        tags: tags_opt.clone(),
                        paths: paths_strings.clone(),
                    }).await;

                    if let Err(e) = res {
                        eprintln!("Failed to save archive bookmark: {}", e);
                    }

                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window_weak2.upgrade() {
                            // Update Dashboard pending state
                            let dash = window.global::<DashboardLogic>();
                            dash.set_has_pending_archive(true);
                            let shared_paths: Vec<SharedString> = paths_strings.into_iter().map(SharedString::from).collect();
                            dash.set_pending_archive_paths(std::rc::Rc::new(slint::VecModel::from(shared_paths)).into());

                            // Switch back to dashboard
                            window.global::<AppState>().set_current_view(SharedString::from("dashboard"));
                            dash.set_terminal_text("Archive configuration saved. You can now press BACKUP NOW.".into());
                        }
                    });
                });
            }
        }
    });

    // Init Wizard Logic
    let wizard = window.global::<InitWizardLogic>();

    wizard.on_next({
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                let wizard = window.global::<InitWizardLogic>();
                wizard.set_current_step(wizard.get_current_step() + 1);
            }
        }
    });

    wizard.on_back({
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                let wizard = window.global::<InitWizardLogic>();
                let current = wizard.get_current_step();
                if current > 0 {
                    wizard.set_current_step(current - 1);
                }
            }
        }
    });

    wizard.on_finish({
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                let wizard = window.global::<InitWizardLogic>();
                let repo_type = wizard.get_repo_type().to_string();
                let repo_name = wizard.get_repo_name().to_string();
                let path_url = wizard.get_path_url().to_string();
                let access_key = wizard.get_access_key().to_string();
                let secret_key = wizard.get_secret_key().to_string();
                let password = wizard.get_password().to_string();
                let retype = wizard.get_retype_password().to_string();

                // Basic validation
                if !password.is_empty() && password != retype {
                    let dashboard = window.global::<DashboardLogic>();
                    dashboard.set_terminal_text("Error: passwords do not match.".into());
                    return;
                }

                // Show progress in UI
                window.global::<AppState>().set_is_processing(true);
                window.global::<AppState>().set_progress(0.1);
                let dashboard = window.global::<DashboardLogic>();
                dashboard.set_terminal_text(format!(
                    "$ borg init --repo {}\nInitializing repository '{}' at {}...",
                    if repo_type == "local" { "--local" } else { "--remote" },
                    repo_name,
                    path_url
                ).into());

                let window_weak_2 = window_weak.clone();
                let state_arc = state_clone.clone();
                tokio::spawn(async move {
                    // Check if name already exists
                    let exists = {
                        let s = state_arc.lock().unwrap();
                        if let Some(db) = &s.db {
                            db.repo_name_exists(&repo_name).await.unwrap_or(false)
                        } else {
                            false
                        }
                    };

                    if exists {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = window_weak_2.upgrade() {
                                w.global::<AppState>().set_is_processing(false);
                                w.global::<DashboardLogic>().set_terminal_text(format!("Error: Repository name '{}' already exists. Please choose a different name.", repo_name).into());
                            }
                        });
                        return;
                    }

                    // Small staged progress updates
                    let _ = slint::invoke_from_event_loop({
                        let window_weak = window_weak_2.clone();
                        move || if let Some(w) = window_weak.upgrade() { w.global::<AppState>().set_progress(0.3); }
                    });

                    let result = commands::init_repository(
                        &repo_type,
                        &repo_name,
                        &path_url,
                        if access_key.is_empty() { None } else { Some(access_key.as_str()) },
                        if secret_key.is_empty() { None } else { Some(secret_key.as_str()) },
                        if password.is_empty() { None } else { Some(password.as_str()) },
                    ).await;

                    let _ = slint::invoke_from_event_loop({
                        let window_weak = window_weak_2.clone();
                        let repo_name_c = repo_name.clone();
                        let path_url_c = path_url.clone();
                        let repo_type_c = repo_type.clone();
                        let password_c = if password.is_empty() { None } else { Some(password.clone()) };
                        
                        move || {
                            if let Some(w) = window_weak.upgrade() {
                                match result {
                                    Ok(()) => {
                                        // Update Rust State
                                        let state_arc_2 = state_arc.clone();
                                        let repo_type_c_2 = repo_type_c.clone();
                                        let repo_name_c_spawn = repo_name_c.clone();
                                        let path_url_c_spawn = path_url_c.clone();
                                        tokio::spawn(async move {
                                            let _ = crate::app_state::BorgAppState::add_bookmark_async(state_arc_2, RepoBookmark {
                                                id: None,
                                                name: repo_name_c_spawn,
                                                path: path_url_c_spawn,
                                                repo_type: repo_type_c_2,
                                            }, password_c).await;
                                        });

                                        w.global::<AppState>().set_progress(1.0);
                                        w.global::<AppState>().set_is_processing(false);
                                        let dash = w.global::<DashboardLogic>();
                                        dash.set_terminal_text(format!("Repository '{}' initialized successfully at {}.", repo_name_c, path_url_c).into());
                                        
                                        // Update UI Model for sidebar
                                        let old_repos = dash.get_repositories();
                                        let new_repos = std::rc::Rc::new(slint::VecModel::default());
                                        for i in 0..old_repos.row_count() {
                                            if let Some(r) = old_repos.row_data(i) {
                                                new_repos.push(r);
                                            }
                                        }
                                        new_repos.push(RepoItem {
                                            name: repo_name_c.into(),
                                            path: path_url_c.into(),
                                            repo_type: repo_type_c.into(),
                                        });
                                        dash.set_repositories(new_repos.into());

                                        // Navigate back to dashboard
                                        w.global::<AppState>().set_current_view(SharedString::from("dashboard"));
                                    }
                                    Err(e) => {
                                        w.global::<AppState>().set_is_processing(false);
                                        let dash = w.global::<DashboardLogic>();
                                        dash.set_terminal_text(format!("Initialization failed: {}", e).into());
                                    }
                                }
                            }
                        }
                    });
                });
            }
        }
    });

    wizard.on_cancel({
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                window.global::<AppState>().set_current_view(SharedString::from("dashboard"));
            }
        }
    });

    wizard.on_browse_path({
        let window_weak = window_weak.clone();
        move || {
            let window_weak2 = window_weak.clone();
            tokio::task::spawn_blocking(move || {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    let path_str = dir.to_string_lossy().to_string();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window_weak2.upgrade() {
                            let wizard = window.global::<InitWizardLogic>();
                            wizard.set_path_url(SharedString::from(path_str));
                        }
                    });
                }
            });
        }
    });

    // Restore Logic
    let restore = window.global::<RestoreLogic>();

    restore.on_start_restore({
        let window_weak = window_weak.clone();
        let state_clone = state.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                let dashboard = window.global::<DashboardLogic>();
                let active_index = dashboard.get_active_repo_index();

                let (repo_path, repo_password) = {
                    let s = state_clone.lock().unwrap();
                    if let Some(bm) = s.bookmarks.get(active_index as usize) {
                        let pwd = s.session_passwords.get(&bm.path).cloned();
                        (bm.path.clone(), pwd)
                    } else {
                        return;
                    }
                };

                let restore_g = window.global::<RestoreLogic>();
                let archive_name = restore_g.get_selected_archive().to_string();
                let use_original = restore_g.get_use_original_paths();
                let dest_text = restore_g.get_restore_path().to_string();

                if !use_original && dest_text.trim().is_empty() {
                    dashboard.set_terminal_text("Please choose a destination folder for restore.".into());
                    return;
                }

                window.global::<AppState>().set_is_processing(true);
                window.global::<AppState>().set_progress(0.05);
                window.global::<AppState>().set_current_view(SharedString::from("dashboard"));
                dashboard.set_terminal_text(format!(
                    "$ borg extract -r {} {}{}\nStarting restore...",
                    repo_path,
                    archive_name,
                    if use_original { " (to ::defaults)" } else { "" }
                ).into());

                let progress = GuiRestoreProgress {
                    window_weak: window_weak.clone(),
                    totals: std::sync::Arc::new(std::sync::Mutex::new((0, 0))),
                    file_sizes: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
                };

                let window_weak2 = window_weak.clone();
                let repo_path_clone = repo_path.clone();
                tokio::spawn(async move {
                    let res = crate::commands::restore_archive(
                        &repo_path_clone,
                        repo_password.as_deref(),
                        &archive_name,
                        use_original,
                        if use_original { None } else { Some(dest_text) },
                        Some(Box::new(progress)),
                    ).await;

                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = window_weak2.upgrade() {
                            match res {
                                Ok(()) => {
                                    w.global::<AppState>().set_is_processing(false);
                                    w.global::<AppState>().set_progress(1.0);
                                    w.global::<DashboardLogic>().set_terminal_text("Restore completed successfully.".into());
                                }
                                Err(e) => {
                                    let err_msg = e.to_string();
                                    if err_msg.contains("Invalid passphrase") || err_msg.contains("Passphrase required") {
                                        w.global::<AppState>().set_pending_auth_action(SharedString::from("restore"));
                                        w.global::<AppState>().set_show_password_dialog(true);
                                        w.global::<AppState>().set_password_dialog_message(format!("Authentication failed for {}. Please enter passphrase:", repo_path_clone).into());
                                        w.global::<DashboardLogic>().set_terminal_text(format!("Authentication failed: {}", e).into());
                                    } else {
                                        w.global::<AppState>().set_is_processing(false);
                                        w.global::<AppState>().set_progress(0.0);
                                        w.global::<DashboardLogic>().set_terminal_text(format!("Restore failed: {}", e).into());
                                    }
                                }
                            }
                        }
                    });
                });
            }
        }
    });

    restore.on_cancel({
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                window.global::<AppState>().set_current_view(SharedString::from("dashboard"));
            }
        }
    });

    restore.on_browse_path({
        let window_weak = window_weak.clone();
        move || {
            let window_weak2 = window_weak.clone();
            tokio::task::spawn_blocking(move || {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    let path_str = dir.to_string_lossy().to_string();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window_weak2.upgrade() {
                            let restore = window.global::<RestoreLogic>();
                            restore.set_restore_path(SharedString::from(path_str));
                        }
                    });
                }
            });
        }
    });

    scheduler_bridge::init_scheduler_bridge(window, state.clone());

    // Archive Content Logic (dialog)
    let content_logic = window.global::<ArchiveContentLogic>();
    content_logic.on_close({
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                window.global::<ArchiveContentLogic>().set_show_dialog(false);
            }
        }
    });

    let archive_files_logic = window.global::<ArchiveFilesLogic>();
    archive_files_logic.on_request_files({
        let window_weak = window_weak.clone();
        let state_clone = state.clone();
        move |archive_name: SharedString, search_term: SharedString, use_regex: bool| {
            if let Some(window) = window_weak.upgrade() {
                let dashboard = window.global::<DashboardLogic>();
                let active_index = dashboard.get_active_repo_index();

                let (repo_path, repo_password) = {
                    let s = state_clone.lock().unwrap();
                    if let Some(bm) = s.bookmarks.get(active_index as usize) {
                        let pwd = s.session_passwords.get(&bm.path).cloned();
                        (bm.path.clone(), pwd)
                    } else {
                        return;
                    }
                };

                let window_weak2 = window_weak.clone();
                tokio::spawn(async move {
                    let res = async {
                        let storage = borg_core::storage::StorageConfig::Local { path: std::path::PathBuf::from(&repo_path) };
                        let op = borg_core::storage::build_operator(storage).map_err(|e| anyhow::anyhow!(e.to_string()))?;
                        let repo = borg_core::repository::Repository::open(op, repo_path.clone(), repo_password.as_deref())
                            .await
                            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                        let restorer = borg_core::archive::ArchiveRestorer::new(&repo);
                        let archive = restorer.load_archive(&archive_name).await
                            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                        Ok::<_, anyhow::Error>(archive)
                    }.await;

                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = window_weak2.upgrade() {
                            match res {
                                Ok(archive) => {
                                    let files_logic = w.global::<ArchiveFilesLogic>();
                                    let original_roots = archive.metadata.original_paths.unwrap_or_default();

                                    let mut files: Vec<SharedString> = archive.items.into_iter()
                                        .map(|item| {
                                            let abs_path = if let Some(root) = original_roots.first() {
                                                let mut p = root.clone();
                                                p.push(&item.path);
                                                p.to_string_lossy().to_string()
                                            } else {
                                                item.path.to_string_lossy().to_string()
                                            };
                                            abs_path.into()
                                        })
                                        .collect();

                                    if !search_term.is_empty() {
                                        if use_regex {
                                            if let Ok(re) = regex::Regex::new(&search_term) {
                                                files.retain(|f| re.is_match(f));
                                            }
                                        } else {
                                            files.retain(|f| f.contains(search_term.as_str()));
                                        }
                                    }
                                    
                                    let initial_load_size = 4096;
                                    if files.len() > initial_load_size {
                                        files.truncate(initial_load_size);
                                    }

                                    files_logic.set_file_list(std::rc::Rc::new(slint::VecModel::from(files)).into());
                                }
                                Err(e) => {
                                    w.global::<DashboardLogic>().set_terminal_text(format!("Error loading archive: {}", e).into());
                                }
                            }
                        }
                    });
                });
            }
        }
    });

    // This is the handler for the double click
    let dash = window.global::<DashboardLogic>();
    dash.on_archive_double_clicked({
        let window_weak = window_weak.clone();
        move |entry| {
            if let Some(window) = window_weak.upgrade() {
                println!("Archive double clicked: {}", entry.name);
                let files_logic = window.global::<ArchiveFilesLogic>();
                files_logic.set_current_archive_name(entry.name.clone());
                files_logic.invoke_request_files(entry.name, "".into(), false);
                files_logic.set_show_archive_files_dialog(true);
            }
        }
    });
}

