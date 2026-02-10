use crate::app_state::BorgAppState as RustAppState;
use crate::app_state::RepoBookmark;
use crate::commands;
use crate::commands::{BackupProgress, RestoreProgress};
use crate::{ArchiveContentLogic, ArchiveEntry, ArchiveFilesLogic, ArchiveContentEntry};
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

    fn on_progress(&self, processed: u64, total: u64) {
        let progress = if total > 0 {
            processed as f32 / total as f32
        } else {
            1.0
        };
        let window_weak = self.window_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = window_weak.upgrade() {
                window.global::<AppState>().set_progress(progress);
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
                    let s = state_clone.lock().unwrap();
                    if let Some(bm) = s.bookmarks.get(active_index as usize) {
                        bm.path.clone()
                    } else {
                        return;
                    }
                };

                // Store in session
                {
                    let mut s = state_clone.lock().unwrap();
                    s.session_passwords.insert(repo_path.clone(), password.to_string());
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
                            } else {
                                w.global::<DashboardLogic>().invoke_backup_clicked();
                            }
                            app.set_pending_auth_action(SharedString::from(""));
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
                
                tokio::spawn(async move {
                    let res = crate::commands::create_archive(
                        &repo_path_clone,
                        repo_password.as_deref(),
                        &name,
                        bm.paths.clone(),
                        &bm.compression,
                        bm.comment.clone(),
                        tags_vec,
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
                                        w.global::<DashboardLogic>().set_terminal_text(format!("Task queued: {}", err_msg).into());
                                        // Here we would add the task to a queue
                                    } else if err_msg.contains("Invalid passphrase") || err_msg.contains("Passphrase required") {
                                        w.global::<AppState>().set_pending_auth_action(SharedString::from("backup"));
                                        w.global::<AppState>().set_show_password_dialog(true);
                                        w.global::<AppState>().set_password_dialog_message(format!("Authentication failed for {}. Please enter passphrase:", repo_path_clone).into());
                                        w.global::<DashboardLogic>().set_terminal_text(format!("Authentication failed: {}", e).into());
                                    } else {
                                        w.global::<AppState>().set_is_processing(false);
                                        w.global::<AppState>().set_progress(0.0);
                                        w.global::<DashboardLogic>().set_terminal_text(format!("Backup failed: {}", e).into());
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
                dashboard.set_active_repo_index(index);
                dashboard.set_has_selected_repo(true);
                dashboard.set_is_scheduled_tasks_active(false);
                window.global::<AppState>().set_current_view("dashboard".into());

                let (repo_path, bookmarks) = {
                    let s = state_clone.lock().unwrap();
                    let path = s.bookmarks.get(index as usize).map(|bm| bm.path.clone());
                    (path, s.bookmarks.clone())
                };

                // Clear current archives while loading
                let empty_model = std::rc::Rc::new(slint::VecModel::from(vec![]));
                dashboard.set_archives(empty_model.into());
                
                if let Some(path) = repo_path {
                    let repo_path_str = path.clone();
                    // Update pending archive state for this repo
                    let (bookmark, password) = {
                        let s = state_clone.lock().unwrap();
                        let bm = s.get_archive_bookmark_for_repo(&path);
                        let pwd = s.session_passwords.get(&path).cloned()
                            .or_else(|| s.get_password(&path).ok());
                        (bm, pwd)
                    };
                    
                    if let Some(bm) = bookmark {
                        dashboard.set_has_pending_archive(true);
                        let paths: Vec<SharedString> = bm.paths.into_iter().map(SharedString::from).collect();
                        dashboard.set_pending_archive_paths(std::rc::Rc::new(slint::VecModel::from(paths)).into());
                    } else {
                        dashboard.set_has_pending_archive(false);
                    }

                    // Load archives
                    let window_weak2 = window_weak.clone();
                    dashboard.set_terminal_text(format!("Loading archives for {}...", repo_path_str).into());
                    
                    tokio::spawn(async move {
                        match async {
                            let storage = borg_core::storage::StorageConfig::Local {
                                path: std::path::PathBuf::from(&repo_path_str)
                            };
                            let op = borg_core::storage::build_operator(storage)
                                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                            let repo = borg_core::repository::Repository::open(
                                op,
                                repo_path_str.clone(),
                                password.as_deref()
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
                                                    hostname: "".into(),
                                                    comment: "".into(),
                                                    tags: "".into(),
                                                }
                                            })
                                            .collect();
                                        let archives_model = std::rc::Rc::new(slint::VecModel::from(archive_entries));
                                        w.global::<DashboardLogic>().set_archives(archives_model.into());
                                        w.global::<DashboardLogic>().set_terminal_text(format!("Archives loaded for {}", repo_path_str).into());
                                    }
                                });
                            }
                            Err(e) => {
                                let error_msg = format!("Failed to load archives: {}", e);
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(w) = window_weak2.upgrade() {
                                        w.global::<DashboardLogic>()
                                            .set_terminal_text(error_msg.clone().into());
                                            
                                        if error_msg.contains("Invalid passphrase") {
                                                w.global::<AppState>().set_pending_auth_action(SharedString::from("load_archives"));
                                                w.global::<AppState>().set_show_password_dialog(true);
                                                w.global::<AppState>().set_password_dialog_message(format!("Authentication failed for {}. Please enter passphrase:", repo_path_str).into());
                                        }
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

    dashboard.on_archive_double_clicked({
        let window_weak = window_weak.clone();
        let state_clone = state.clone();
        move |entry| {
            if let Some(window) = window_weak.upgrade() {
                let dashboard = window.global::<DashboardLogic>();
                let active_index = dashboard.get_active_repo_index();
                let archive_name = entry.name.to_string();

                let (repo_path, repo_password) = {
                    let s = state_clone.lock().unwrap();
                    if let Some(bm) = s.bookmarks.get(active_index as usize) {
                        let pwd = s.session_passwords.get(&bm.path).cloned();
                        (bm.path.clone(), pwd)
                    } else {
                        return;
                    }
                };

                let content_logic = window.global::<ArchiveContentLogic>();
                content_logic.set_archive_name(entry.name);
                content_logic.set_show_dialog(true);
                // Clear previous list
                content_logic.set_files(std::rc::Rc::new(slint::VecModel::default()).into());
                content_logic.set_search_text("Loading...".into());

                let window_weak2 = window_weak.clone();
                let repo_path_clone = repo_path.clone();
                let archive_name_clone = archive_name.clone();
                
                let state_clone = state_clone.clone();
                tokio::spawn(async move {
                    let res = crate::commands::list_archive_files(
                        &repo_path_clone,
                        repo_password.as_deref(),
                        &archive_name_clone
                    ).await;

                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = window_weak2.upgrade() {
                            match res {
                                Ok(files) => {
                                    // Cache files
                                    {
                                        let mut s = state_clone.lock().unwrap();
                                        s.archive_cache = Some((archive_name_clone.clone(), files.as_slice().iter().map(|f| crate::commands::FileEntry {
                                            path: f.path.clone(),
                                            size: f.size,
                                            is_dir: f.is_dir,
                                            mode: f.mode,
                                            user: f.user.clone(),
                                            group: f.group.clone(),
                                            mtime: f.mtime
                                        }).collect())); 
                                        // Wait, cannot clone Vec<FileEntry> easily unless Clone derived? 
                                        // I defined FileEntry in commands.rs without derive Clone. Let's fix that or manually clone.
                                        // Manual map is fine.
                                    }

                                    let content_logic = w.global::<ArchiveContentLogic>();
                                    content_logic.set_search_text("".into());
                                    
                                    // Initial render (all files)
                                    let mut ui_files = Vec::new();
                                    // Limit initial display to 1000 items to avoid freezing UI if massive archive
                                    // Or implement virtualization. ListView handles virtualization well but model update can be slow.
                                    // Let's take all for now.
                                    for f in files {
                                        let icon_path = if f.is_dir {
                                            slint::Image::load_from_path(std::path::Path::new("assets/folder.svg")).unwrap_or_default()
                                        } else {
                                            slint::Image::load_from_path(std::path::Path::new("assets/file.svg")).unwrap_or_default()
                                        };
                                        
                                        ui_files.push(ArchiveContentEntry {
                                            path: f.path.into(),
                                            size: crate::commands::human_bytes(f.size).into(),
                                            item_type: if f.is_dir { "d".into() } else { "f".into() },
                                            user: f.user.into(),
                                            group: f.group.into(),
                                            mtime: chrono::DateTime::from_timestamp(f.mtime, 0).unwrap_or_default().format("%Y-%m-%d %H:%M").to_string().into(),
                                            icon: icon_path, // Need to load image
                                            selected: false,
                                        });
                                    }
                                    let model = std::rc::Rc::new(slint::VecModel::from(ui_files));
                                    content_logic.set_files(model.into());
                                }
                                Err(e) => {
                                    let msg = format!("Error loading archive: {}", e);
                                    w.global::<DashboardLogic>().set_terminal_text(msg.clone().into());
                                    // Show error in search bar
                                    w.global::<ArchiveContentLogic>().set_search_text(msg.into());
                                }
                            }
                        }
                    });
                });
            }
        }
    });

    // ArchiveContentLogic
    let content_logic = window.global::<ArchiveContentLogic>();
    
    content_logic.on_close({
        let window_weak = window_weak.clone();
        move || {
            if let Some(window) = window_weak.upgrade() {
                window.global::<ArchiveContentLogic>().set_show_dialog(false);
            }
        }
    });

    content_logic.on_search_apply({
        let window_weak = window_weak.clone();
        let state_clone = state.clone();
        move |term, regex| {
            if let Some(window) = window_weak.upgrade() {
                 let s = state_clone.lock().unwrap();
                 if let Some((_, files)) = &s.archive_cache {
                     let term_lower = term.to_lowercase();
                     let mut ui_files = Vec::new();
                     
                     for f in files {
                         let matches = if regex {
                             // Simple regex or contains
                             f.path.to_lowercase().contains(&term_lower) // Placeholder for real regex
                         } else {
                             f.path.to_lowercase().contains(&term_lower)
                         };

                         if matches {
                            let icon_path = if f.is_dir {
                                slint::Image::load_from_path(std::path::Path::new("assets/folder.svg")).unwrap_or_default()
                            } else {
                                slint::Image::load_from_path(std::path::Path::new("assets/file.svg")).unwrap_or_default()
                            };

                             ui_files.push(ArchiveContentEntry {
                                path: f.path.clone().into(),
                                size: crate::commands::human_bytes(f.size).into(),
                                item_type: if f.is_dir { "d".into() } else { "f".into() },
                                user: f.user.clone().into(),
                                group: f.group.clone().into(),
                                mtime: chrono::DateTime::from_timestamp(f.mtime, 0).unwrap_or_default().format("%Y-%m-%d %H:%M").to_string().into(),
                                icon: icon_path,
                                selected: false,
                             });
                         }
                     }
                     let model = std::rc::Rc::new(slint::VecModel::from(ui_files));
                     window.global::<ArchiveContentLogic>().set_files(model.into());
                 }
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
                {
                    let mut s = state_clone.lock().unwrap();
                    s.upsert_archive_bookmark(crate::app_state::ArchiveBookmark {
                        repo_name,
                        repo_path: repo_path.clone(),
                        compression: compression.clone(),
                        redundancy,
                        use_custom_name,
                        archive_name: archive_name_opt.clone(),
                        comment: comment_opt.clone(),
                        tags: tags_opt.clone(),
                        paths: paths_strings.clone(),
                    });
                }

                // Update Dashboard pending state
                let dash = window.global::<DashboardLogic>();
                dash.set_has_pending_archive(true);
                let shared_paths: Vec<SharedString> = paths_strings.into_iter().map(SharedString::from).collect();
                dash.set_pending_archive_paths(std::rc::Rc::new(slint::VecModel::from(shared_paths)).into());

                // Switch back to dashboard
                window.global::<AppState>().set_current_view(SharedString::from("dashboard"));
                dash.set_terminal_text("Archive configuration saved. You can now press BACKUP NOW.".into());
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
                                        {
                                            let mut s = state_arc.lock().unwrap();
                                            s.add_bookmark(RepoBookmark {
                                                name: repo_name_c.clone(),
                                                path: path_url_c.clone(),
                                                repo_type: repo_type_c.clone(),
                                            }, password_c);
                                        }

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
                                    
                                    let mut loaded_files = files.len();
                                    let initial_load_size = 4096;
                                    if files.len() > initial_load_size {
                                        files.truncate(initial_load_size);
                                        loaded_files = initial_load_size;
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
                let files_logic = window.global::<ArchiveFilesLogic>();
                files_logic.set_current_archive_name(entry.name.clone());
                files_logic.invoke_request_files(entry.name, "".into(), false);
                files_logic.set_show_archive_files_dialog(true);
            }
        }
    });
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.1}T", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1}G", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}M", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}K", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}
