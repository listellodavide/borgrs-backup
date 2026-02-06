//! List archives/files command
//!
//! Lists archives in a repository or files within a specific archive.
//! Supports filtering, sorting, and various output formats.

use anyhow::Result;
use borg_core::archive::{ArchiveExtractor, ArchiveItem, ItemType};
use borg_core::repository::ArchiveRef;
use serde::Serialize;

use super::{get_repo_path, open_repository};
use crate::{Cli, ListArgs};

pub async fn run(cli: &Cli, args: &ListArgs) -> Result<()> {
    let repo_path_raw = get_repo_path(cli)?;

    // Normalize WebDAV URL if credentials are provided (either from CLI or if it's a remote repo)
    let repo_path = if args.webdav_user.is_some() || args.webdav_pass.is_some() {
        let scheme = if cli.remote_repo.is_some() {
            if repo_path_raw.starts_with("https") || repo_path_raw.starts_with("webdavs") || repo_path_raw.starts_with("davs") {
                "webdavs"
            } else {
                "webdav"
            }
        } else {
            "webdav"
        };
        crate::commands::init::normalize_webdav_url(
            &repo_path_raw,
            scheme,
            args.webdav_user.as_deref(),
            args.webdav_pass.as_deref(),
        )?
    } else {
        repo_path_raw
    };

    let repo = open_repository(&repo_path).await?;

    match &args.archive {
        Some(archive) => {
            list_archive_contents(&repo, archive, args).await?;
        }
        None => {
            list_archives(&repo, args).await?;
        }
    }

    Ok(())
}

/// List all archives in the repository
async fn list_archives(repo: &borg_core::repository::Repository, args: &ListArgs) -> Result<()> {
    let manifest = repo.load_manifest().await?;
    let mut archives: Vec<_> = manifest.archives.iter().collect();

    // Apply filters
    if let Some(ref pattern) = args.pattern {
        archives.retain(|a| a.name.contains(pattern));
    }

    // Sort archives
    match args.sort_by.as_deref() {
        Some("name") => archives.sort_by(|a, b| a.name.cmp(&b.name)),
        Some("timestamp") | Some("time") => archives.sort_by(|a, b| b.time.cmp(&a.time)),
        _ => archives.sort_by(|a, b| b.time.cmp(&a.time)), // Default: newest first
    }

    // Apply first/last limits
    if let Some(first) = args.first {
        archives.truncate(first);
    }
    if let Some(last) = args.last {
        let len = archives.len();
        if last < len {
            archives = archives.into_iter().skip(len - last).collect();
        }
    }

    // Output
    if args.json {
        print_archives_json(&archives)?;
    } else if args.short {
        for archive in &archives {
            println!("{}", archive.name);
        }
    } else {
        print_archives_table(&archives);
    }

    Ok(())
}

/// List contents of a specific archive
async fn list_archive_contents(
    repo: &borg_core::repository::Repository,
    archive_name: &str,
    args: &ListArgs,
) -> Result<()> {
    let extractor = ArchiveExtractor::new(repo);
    let archive = extractor.load_archive(archive_name).await?;

    let mut items: Vec<_> = archive.items.iter().collect();

    // Apply path filters
    if !args.paths.is_empty() {
        items.retain(|item| {
            args.paths.iter().any(|p| item.path.starts_with(p))
        });
    }

    // Apply pattern filter
    if let Some(ref pattern) = args.pattern {
        let pattern_lower = pattern.to_lowercase();
        items.retain(|item| {
            item.path.to_string_lossy().to_lowercase().contains(&pattern_lower)
        });
    }

    // Apply exclusions
    for exclude in &args.exclude {
        let exclude_lower = exclude.to_lowercase();
        items.retain(|item| {
            !item.path.to_string_lossy().to_lowercase().contains(&exclude_lower)
        });
    }

    // Sort items
    match args.sort_by.as_deref() {
        Some("size") => items.sort_by(|a, b| b.size.cmp(&a.size)),
        Some("timestamp") | Some("time") => items.sort_by(|a, b| b.attrs.mtime.cmp(&a.attrs.mtime)),
        Some("name") | _ => items.sort_by(|a, b| a.path.cmp(&b.path)),
    }

    // Apply first/last limits
    if let Some(first) = args.first {
        items.truncate(first);
    }
    if let Some(last) = args.last {
        let len = items.len();
        if last < len {
            items = items.into_iter().skip(len - last).collect();
        }
    }

    // Output
    if args.json {
        print_items_json(&items)?;
    } else if args.short {
        for item in &items {
            println!("{}", item.path.display());
        }
    } else {
        print_items_table(&items, args.format.as_deref());
    }

    Ok(())
}

/// Print archives in JSON format
fn print_archives_json(archives: &[&ArchiveRef]) -> Result<()> {
    #[derive(Serialize)]
    struct ArchiveJson {
        name: String,
        time: String,
        id: String,
    }

    let json_archives: Vec<_> = archives
        .iter()
        .map(|a| ArchiveJson {
            name: a.name.clone(),
            time: a.time.to_rfc3339(),
            id: a.id.to_hex(),
        })
        .collect();

    let json = serde_json::to_string_pretty(&json_archives)?;
    println!("{}", json);
    Ok(())
}

/// Print archives in table format
fn print_archives_table(archives: &[&ArchiveRef]) {
    if archives.is_empty() {
        println!("No archives found.");
        return;
    }

    println!("Archives ({}):", archives.len());
    println!();
    println!("{:<40} {:<25} {}", "Name", "Time", "ID");
    println!("{}", "-".repeat(90));

    for archive in archives {
        let time_str = archive.time.format("%Y-%m-%d %H:%M:%S").to_string();
        let id_short = &archive.id.to_hex()[..16];
        println!("{:<40} {:<25} {}...", archive.name, time_str, id_short);
    }

    println!();
    println!("Total: {} archives", archives.len());
}

/// Print archive items in JSON format
fn print_items_json(items: &[&ArchiveItem]) -> Result<()> {
    #[derive(Serialize)]
    struct ItemJson {
        path: String,
        #[serde(rename = "type")]
        item_type: String,
        size: u64,
        mode: u32,
        uid: u32,
        gid: u32,
        mtime: i64,
    }

    let json_items: Vec<_> = items
        .iter()
        .map(|item| ItemJson {
            path: item.path.to_string_lossy().to_string(),
            item_type: format!("{:?}", item.item_type).to_lowercase(),
            size: item.size,
            mode: item.attrs.mode,
            uid: item.attrs.uid,
            gid: item.attrs.gid,
            mtime: item.attrs.mtime,
        })
        .collect();

    let json = serde_json::to_string_pretty(&json_items)?;
    println!("{}", json);
    Ok(())
}

/// Print archive items in table format
fn print_items_table(items: &[&ArchiveItem], format: Option<&str>) {
    if items.is_empty() {
        println!("No items found.");
        return;
    }

    let detailed = format == Some("detailed") || format == Some("long");

    // Calculate totals
    let total_files = items.iter().filter(|i| i.item_type == ItemType::File).count();
    let total_dirs = items.iter().filter(|i| i.item_type == ItemType::Directory).count();
    let total_size: u64 = items.iter().map(|i| i.size).sum();

    if detailed {
        println!("{:<10} {:>10} {:>6} {:>6} {:>19} {}", 
            "Mode", "Size", "UID", "GID", "Modified", "Path");
        println!("{}", "-".repeat(100));

        for item in items {
            let type_char = get_type_char(item.item_type);
            let mode_str = format_mode(type_char, item.attrs.mode);
            let size_str = format_size(item.size);
            let mtime = chrono::DateTime::from_timestamp(item.attrs.mtime, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "unknown".to_string());

            println!(
                "{:<10} {:>10} {:>6} {:>6} {:>19} {}",
                mode_str,
                size_str,
                item.attrs.uid,
                item.attrs.gid,
                mtime,
                item.path.display()
            );
        }
    } else {
        for item in items {
            let type_char = get_type_char(item.item_type);
            let size_str = if item.item_type == ItemType::File {
                format_size(item.size)
            } else {
                "-".to_string()
            };

            println!(
                "{} {:>10}  {}",
                type_char,
                size_str,
                item.path.display()
            );
        }
    }

    println!();
    println!(
        "Total: {} files, {} directories, {}",
        total_files,
        total_dirs,
        format_size(total_size)
    );
}

/// Get type character for display
fn get_type_char(item_type: ItemType) -> char {
    match item_type {
        ItemType::File => '-',
        ItemType::Directory => 'd',
        ItemType::Symlink => 'l',
        ItemType::Hardlink => 'h',
        ItemType::BlockDevice => 'b',
        ItemType::CharDevice => 'c',
        ItemType::Fifo => 'p',
        ItemType::Socket => 's',
    }
}

/// Format Unix mode bits
fn format_mode(type_char: char, mode: u32) -> String {
    let user = format_rwx((mode >> 6) & 7);
    let group = format_rwx((mode >> 3) & 7);
    let other = format_rwx(mode & 7);
    format!("{}{}{}{}", type_char, user, group, other)
}

/// Format rwx permission bits
fn format_rwx(bits: u32) -> String {
    let r = if bits & 4 != 0 { 'r' } else { '-' };
    let w = if bits & 2 != 0 { 'w' } else { '-' };
    let x = if bits & 1 != 0 { 'x' } else { '-' };
    format!("{}{}{}", r, w, x)
}

/// Format size in human-readable format
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
