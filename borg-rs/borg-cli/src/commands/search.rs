//! Search command for finding files across archives
//!
//! Provides full-text search capabilities for finding files in backup archives
//! by path pattern, file type, size, and modification time.

use anyhow::Result;
use borg_core::archive::{ArchiveRestorer, ItemType};
use borg_core::catalog::{SearchQuery, SearchResult};

use super::{get_repo_path, open_repository};
use crate::Cli;

/// Arguments for the search command
#[derive(Debug, clap::Args)]
pub struct SearchArgs {
    /// Search pattern (path substring or glob with --glob)
    #[arg(required = true)]
    pattern: String,

    /// Search only in specific archive
    #[arg(short, long)]
    archive: Option<String>,

    /// WebDAV username
    #[arg(long, value_name = "USER")]
    pub webdav_user: Option<String>,

    /// WebDAV password
    #[arg(long, value_name = "PASS")]
    pub webdav_pass: Option<String>,

    /// Use glob pattern matching (*, ?, etc.)
    #[arg(long)]
    glob: bool,

    /// Use exact path matching
    #[arg(long)]
    exact: bool,

    /// Filter by file type (file, dir, symlink)
    #[arg(long, short = 't')]
    file_type: Option<String>,

    /// Minimum file size (e.g., "1M", "100K")
    #[arg(long)]
    min_size: Option<String>,

    /// Maximum file size (e.g., "1G", "500M")
    #[arg(long)]
    max_size: Option<String>,

    /// Modified after date (YYYY-MM-DD)
    #[arg(long)]
    newer: Option<String>,

    /// Modified before date (YYYY-MM-DD)
    #[arg(long)]
    older: Option<String>,

    /// Maximum number of results
    #[arg(long, short = 'n', default_value = "100")]
    limit: usize,

    /// JSON output format
    #[arg(long)]
    json: bool,

    /// Show full file details
    #[arg(long, short = 'l')]
    long: bool,
}

pub async fn run(cli: &Cli, args: &SearchArgs) -> Result<()> {
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

    // Build search query
    let mut query = SearchQuery::new()
        .path(&args.pattern)
        .limit(args.limit);

    if args.glob {
        query = query.glob();
    }

    if args.exact {
        query = query.exact();
    }

    if let Some(ref archive) = args.archive {
        query = query.in_archive(archive);
    }

    if let Some(ref file_type) = args.file_type {
        let item_type = parse_file_type(file_type)?;
        query = query.file_type(item_type);
    }

    if let Some(ref min_size) = args.min_size {
        query = query.min_size(parse_size(min_size)?);
    }

    if let Some(ref max_size) = args.max_size {
        query = query.max_size(parse_size(max_size)?);
    }

    if let Some(ref newer) = args.newer {
        query.modified_after = Some(parse_date(newer)?);
    }

    if let Some(ref older) = args.older {
        query.modified_before = Some(parse_date(older)?);
    }

    // Search using direct archive scanning
    let results = search_archives(&repo, &query).await?;

    // Output results
    if args.json {
        print_json_results(&results)?;
    } else {
        print_text_results(&results, args.long);
    }

    Ok(())
}

/// Search through archives directly
async fn search_archives(
    repo: &borg_core::repository::Repository,
    query: &SearchQuery,
) -> Result<Vec<SearchResult>> {
    let extractor = ArchiveRestorer::new(repo);
    let manifest = repo.load_manifest().await?;
    let mut results = Vec::new();

    // Determine which archives to search
    let archives_to_search: Vec<_> = if let Some(ref filter) = query.archive_filter {
        manifest.archives.iter()
            .filter(|a| a.name == *filter)
            .collect()
    } else {
        manifest.archives.iter().collect()
    };

    for archive_ref in archives_to_search {
        let archive = extractor.load_archive(&archive_ref.name).await?;

        for item in &archive.items {
            if matches_query(item, query) {
                results.push(SearchResult {
                    archive_name: archive_ref.name.clone(),
                    path: item.path.clone(),
                    item_type: item.item_type,
                    size: item.size,
                    mtime: item.attrs.mtime,
                });

                if let Some(limit) = query.limit {
                    if results.len() >= limit {
                        return Ok(results);
                    }
                }
            }
        }
    }

    Ok(results)
}

/// Check if an item matches the search query
fn matches_query(
    item: &borg_core::archive::ArchiveItem,
    query: &SearchQuery,
) -> bool {
    // Path pattern matching
    if let Some(ref pattern) = query.path_pattern {
        let path_str = item.path.to_string_lossy().to_lowercase();
        let pattern_lower = pattern.to_lowercase();

        let matches = if query.exact_match {
            path_str == pattern_lower
        } else if query.glob_pattern {
            glob_match(&path_str, &pattern_lower)
        } else {
            path_str.contains(&pattern_lower)
        };

        if !matches {
            return false;
        }
    }

    // Item type filter
    if let Some(item_type) = query.item_type {
        if item.item_type != item_type {
            return false;
        }
    }

    // Size filters
    if let Some(min_size) = query.min_size {
        if item.size < min_size {
            return false;
        }
    }
    if let Some(max_size) = query.max_size {
        if item.size > max_size {
            return false;
        }
    }

    // Time filters
    if let Some(after) = query.modified_after {
        if item.attrs.mtime < after {
            return false;
        }
    }
    if let Some(before) = query.modified_before {
        if item.attrs.mtime > before {
            return false;
        }
    }

    true
}

/// Simple glob pattern matching
fn glob_match(text: &str, pattern: &str) -> bool {
    let mut text_chars = text.chars().peekable();
    let mut pattern_chars = pattern.chars().peekable();

    while let Some(pc) = pattern_chars.next() {
        match pc {
            '*' => {
                if pattern_chars.peek().is_none() {
                    return true;
                }
                let rest_pattern: String = pattern_chars.collect();
                let rest_text: String = text_chars.collect();
                for i in 0..=rest_text.len() {
                    if glob_match(&rest_text[i..], &rest_pattern) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                if text_chars.next().is_none() {
                    return false;
                }
            }
            c => {
                if text_chars.next() != Some(c) {
                    return false;
                }
            }
        }
    }

    text_chars.next().is_none()
}

/// Parse file type from string
fn parse_file_type(s: &str) -> Result<ItemType> {
    match s.to_lowercase().as_str() {
        "file" | "f" => Ok(ItemType::File),
        "dir" | "directory" | "d" => Ok(ItemType::Directory),
        "symlink" | "link" | "l" => Ok(ItemType::Symlink),
        "hardlink" | "h" => Ok(ItemType::Hardlink),
        _ => Err(anyhow::anyhow!(
            "Unknown file type: {}. Use: file, dir, symlink, hardlink",
            s
        )),
    }
}

/// Parse size string (e.g., "100M", "1G", "500K")
fn parse_size(s: &str) -> Result<u64> {
    let s = s.trim().to_uppercase();
    
    if s.is_empty() {
        return Err(anyhow::anyhow!("Empty size string"));
    }

    let (num_str, multiplier) = if s.ends_with("TB") || s.ends_with('T') {
        let num = s.trim_end_matches("TB").trim_end_matches('T');
        (num, 1024u64 * 1024 * 1024 * 1024)
    } else if s.ends_with("GB") || s.ends_with('G') {
        let num = s.trim_end_matches("GB").trim_end_matches('G');
        (num, 1024u64 * 1024 * 1024)
    } else if s.ends_with("MB") || s.ends_with('M') {
        let num = s.trim_end_matches("MB").trim_end_matches('M');
        (num, 1024u64 * 1024)
    } else if s.ends_with("KB") || s.ends_with('K') {
        let num = s.trim_end_matches("KB").trim_end_matches('K');
        (num, 1024u64)
    } else if s.ends_with('B') {
        let num = s.trim_end_matches('B');
        (num, 1u64)
    } else {
        (s.as_str(), 1u64)
    };

    let num: f64 = num_str.parse()
        .map_err(|_| anyhow::anyhow!("Invalid size: {}", s))?;

    Ok((num * multiplier as f64) as u64)
}

/// Parse date string (YYYY-MM-DD) to Unix timestamp
fn parse_date(s: &str) -> Result<i64> {
    let date = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("Invalid date format: {}. Use YYYY-MM-DD", s))?;
    
    let datetime = date.and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow::anyhow!("Invalid date: {}", s))?;
    
    Ok(datetime.and_utc().timestamp())
}

/// Print results in JSON format
fn print_json_results(results: &[SearchResult]) -> Result<()> {
    let json = serde_json::to_string_pretty(results)?;
    println!("{}", json);
    Ok(())
}

/// Print results in text format
fn print_text_results(results: &[SearchResult], long_format: bool) {
    if results.is_empty() {
        println!("No matches found.");
        return;
    }

    println!("Found {} matches:", results.len());
    println!();

    // Group by archive if not in long format
    if long_format {
        for result in results {
            let type_char = match result.item_type {
                ItemType::File => 'f',
                ItemType::Directory => 'd',
                ItemType::Symlink => 'l',
                ItemType::Hardlink => 'h',
                ItemType::BlockDevice => 'b',
                ItemType::CharDevice => 'c',
                ItemType::Fifo => 'p',
                ItemType::Socket => 's',
            };

            let mtime = chrono::DateTime::from_timestamp(result.mtime, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "unknown".to_string());

            println!(
                "{} {:>10}  {}  [{}]  {}",
                type_char,
                format_size(result.size),
                mtime,
                result.archive_name,
                result.path.display()
            );
        }
    } else {
        let mut current_archive = String::new();
        
        for result in results {
            if result.archive_name != current_archive {
                if !current_archive.is_empty() {
                    println!();
                }
                println!("Archive: {}", result.archive_name);
                current_archive = result.archive_name.clone();
            }
            println!("  {}", result.path.display());
        }
    }

    println!();
    println!("Total: {} matches", results.len());
}

/// Format size in human-readable format
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1}G", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}M", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}K", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}
