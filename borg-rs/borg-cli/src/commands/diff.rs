//! Compare archives command
//!
//! Compares two archives and shows differences between them including:
//! - Added files (in archive2 but not archive1)
//! - Removed files (in archive1 but not archive2)
//! - Modified files (changed size, time, or content)

use anyhow::Result;
use borg_core::archive::{Archive, ArchiveExtractor, ArchiveItem, ItemType};
use borg_core::catalog::{ArchiveDiff, ChangeType, DiffEntry};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::{get_repo_path, open_repository};
use crate::{Cli, DiffArgs};

pub async fn run(cli: &Cli, args: &DiffArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;
    let repo = open_repository(&repo_path).await?;

    // Load both archives
    let extractor = ArchiveExtractor::new(&repo);
    
    let archive1 = extractor.load_archive(&args.archive1).await?;
    let archive2 = extractor.load_archive(&args.archive2).await?;

    // Compute diff
    let diff = compute_diff(&archive1, &archive2, &args.paths);

    // Output results
    if args.json {
        print_json_diff(&diff)?;
    } else {
        print_text_diff(&diff, &args.sort);
    }

    Ok(())
}

/// Compute differences between two archives
fn compute_diff(archive1: &Archive, archive2: &Archive, filter_paths: &[PathBuf]) -> ArchiveDiff {
    let mut files1: HashMap<PathBuf, &ArchiveItem> = HashMap::new();
    let mut files2: HashMap<PathBuf, &ArchiveItem> = HashMap::new();

    // Build file maps, optionally filtering by paths
    for item in &archive1.items {
        if should_include_path(&item.path, filter_paths) {
            files1.insert(item.path.clone(), item);
        }
    }

    for item in &archive2.items {
        if should_include_path(&item.path, filter_paths) {
            files2.insert(item.path.clone(), item);
        }
    }

    // Collect all unique paths
    let all_paths: HashSet<_> = files1.keys().chain(files2.keys()).cloned().collect();

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();
    let mut unchanged = 0;

    for path in all_paths {
        match (files1.get(&path), files2.get(&path)) {
            (None, Some(item2)) => {
                // File added in archive2
                added.push(DiffEntry {
                    path: path.clone(),
                    change_type: ChangeType::Added,
                    item_type: item2.item_type,
                    size_before: None,
                    size_after: Some(item2.size),
                    mtime_before: None,
                    mtime_after: Some(item2.attrs.mtime),
                });
            }
            (Some(item1), None) => {
                // File removed in archive2
                removed.push(DiffEntry {
                    path: path.clone(),
                    change_type: ChangeType::Removed,
                    item_type: item1.item_type,
                    size_before: Some(item1.size),
                    size_after: None,
                    mtime_before: Some(item1.attrs.mtime),
                    mtime_after: None,
                });
            }
            (Some(item1), Some(item2)) => {
                // Check if modified
                if is_modified(item1, item2) {
                    modified.push(DiffEntry {
                        path: path.clone(),
                        change_type: ChangeType::Modified,
                        item_type: item2.item_type,
                        size_before: Some(item1.size),
                        size_after: Some(item2.size),
                        mtime_before: Some(item1.attrs.mtime),
                        mtime_after: Some(item2.attrs.mtime),
                    });
                } else {
                    unchanged += 1;
                }
            }
            (None, None) => unreachable!(),
        }
    }

    // Sort by path by default
    added.sort_by(|a, b| a.path.cmp(&b.path));
    removed.sort_by(|a, b| a.path.cmp(&b.path));
    modified.sort_by(|a, b| a.path.cmp(&b.path));

    ArchiveDiff {
        archive1: archive1.metadata.name.clone(),
        archive2: archive2.metadata.name.clone(),
        added,
        removed,
        modified,
        unchanged,
    }
}

/// Check if a path should be included based on filter
fn should_include_path(path: &PathBuf, filter_paths: &[PathBuf]) -> bool {
    if filter_paths.is_empty() {
        return true;
    }

    for filter in filter_paths {
        if path.starts_with(filter) || path == filter {
            return true;
        }
    }

    false
}

/// Check if two items are different
fn is_modified(item1: &ArchiveItem, item2: &ArchiveItem) -> bool {
    // Type changed
    if item1.item_type != item2.item_type {
        return true;
    }

    // Size changed (for files)
    if item1.size != item2.size {
        return true;
    }

    // Modification time changed
    if item1.attrs.mtime != item2.attrs.mtime {
        return true;
    }

    // Chunks changed (content difference)
    if item1.chunks != item2.chunks {
        return true;
    }

    // Mode changed
    if item1.attrs.mode != item2.attrs.mode {
        return true;
    }

    false
}

/// Print diff in JSON format
fn print_json_diff(diff: &ArchiveDiff) -> Result<()> {
    let json = serde_json::to_string_pretty(diff)?;
    println!("{}", json);
    Ok(())
}

/// Print diff in text format
fn print_text_diff(diff: &ArchiveDiff, sort_by: &Option<String>) {
    println!(
        "Comparing {} with {}",
        diff.archive1, diff.archive2
    );
    println!();

    // Sort entries if requested
    let mut added = diff.added.clone();
    let mut removed = diff.removed.clone();
    let mut modified = diff.modified.clone();

    if let Some(sort) = sort_by {
        match sort.as_str() {
            "size" => {
                added.sort_by(|a, b| {
                    b.size_after.unwrap_or(0).cmp(&a.size_after.unwrap_or(0))
                });
                removed.sort_by(|a, b| {
                    b.size_before.unwrap_or(0).cmp(&a.size_before.unwrap_or(0))
                });
                modified.sort_by(|a, b| {
                    let delta_a = a.size_after.unwrap_or(0) as i64 - a.size_before.unwrap_or(0) as i64;
                    let delta_b = b.size_after.unwrap_or(0) as i64 - b.size_before.unwrap_or(0) as i64;
                    delta_b.abs().cmp(&delta_a.abs())
                });
            }
            "timestamp" => {
                added.sort_by(|a, b| {
                    b.mtime_after.unwrap_or(0).cmp(&a.mtime_after.unwrap_or(0))
                });
                removed.sort_by(|a, b| {
                    b.mtime_before.unwrap_or(0).cmp(&a.mtime_before.unwrap_or(0))
                });
                modified.sort_by(|a, b| {
                    b.mtime_after.unwrap_or(0).cmp(&a.mtime_after.unwrap_or(0))
                });
            }
            _ => {} // Default is path, already sorted
        }
    }

    // Print added files
    if !added.is_empty() {
        println!("Added ({}):", added.len());
        for entry in &added {
            print_entry('+', entry);
        }
        println!();
    }

    // Print removed files
    if !removed.is_empty() {
        println!("Removed ({}):", removed.len());
        for entry in &removed {
            print_entry('-', entry);
        }
        println!();
    }

    // Print modified files
    if !modified.is_empty() {
        println!("Modified ({}):", modified.len());
        for entry in &modified {
            print_entry('M', entry);
        }
        println!();
    }

    // Print summary
    println!("Summary:");
    println!("  Added:     {} files", diff.added.len());
    println!("  Removed:   {} files", diff.removed.len());
    println!("  Modified:  {} files", diff.modified.len());
    println!("  Unchanged: {} files", diff.unchanged);
    println!("  Total changes: {}", diff.total_changes());

    // Calculate size changes
    let added_size: u64 = diff.added.iter()
        .filter_map(|e| e.size_after)
        .sum();
    let removed_size: u64 = diff.removed.iter()
        .filter_map(|e| e.size_before)
        .sum();
    let size_change: i64 = diff.modified.iter()
        .map(|e| {
            e.size_after.unwrap_or(0) as i64 - e.size_before.unwrap_or(0) as i64
        })
        .sum();

    let total_change = added_size as i64 - removed_size as i64 + size_change;

    println!();
    println!("Size changes:");
    println!("  Added:    +{}", format_size(added_size));
    println!("  Removed:  -{}", format_size(removed_size));
    println!("  Modified: {}{}", if size_change >= 0 { "+" } else { "" }, format_size_signed(size_change));
    println!("  Net:      {}{}", if total_change >= 0 { "+" } else { "" }, format_size_signed(total_change));
}

/// Print a single diff entry
fn print_entry(prefix: char, entry: &DiffEntry) {
    let type_char = match entry.item_type {
        ItemType::File => 'f',
        ItemType::Directory => 'd',
        ItemType::Symlink => 'l',
        ItemType::Hardlink => 'h',
        ItemType::BlockDevice => 'b',
        ItemType::CharDevice => 'c',
        ItemType::Fifo => 'p',
        ItemType::Socket => 's',
    };

    let size_info = match entry.change_type {
        ChangeType::Added => {
            format!("{}", format_size(entry.size_after.unwrap_or(0)))
        }
        ChangeType::Removed => {
            format!("{}", format_size(entry.size_before.unwrap_or(0)))
        }
        ChangeType::Modified => {
            let before = entry.size_before.unwrap_or(0);
            let after = entry.size_after.unwrap_or(0);
            let delta = after as i64 - before as i64;
            if delta != 0 {
                format!(
                    "{} -> {} ({}{})",
                    format_size(before),
                    format_size(after),
                    if delta > 0 { "+" } else { "" },
                    format_size_signed(delta)
                )
            } else {
                format!("{} (content changed)", format_size(after))
            }
        }
    };

    println!(
        "  {} {} {:>12}  {}",
        prefix,
        type_char,
        size_info,
        entry.path.display()
    );
}

/// Format size in human-readable format
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Format signed size
fn format_size_signed(bytes: i64) -> String {
    let abs_bytes = bytes.unsigned_abs();
    let formatted = format_size(abs_bytes);
    if bytes < 0 {
        format!("-{}", formatted)
    } else {
        formatted
    }
}
