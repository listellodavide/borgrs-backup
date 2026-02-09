//! Repository/archive info command

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, InfoArgs};

use borg_core::archive::ArchiveRestorer;

pub async fn run(cli: &Cli, args: &InfoArgs) -> Result<()> {
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
        Some(archive_name) => {
            let restorer = ArchiveRestorer::new(&repo);
            let archive = restorer.load_archive(archive_name).await?;
            
            println!(
                "{:<65} | {:<20} | {:<20} | {:<15} | {:<15} | {:<20}",
                "Archive Name",
                "Comment",
                "Tags",
                "Hostname",
                "Username",
                "Datetime"
            );

            println!("{}", "-".repeat(175));

            let tags = archive.metadata.tags.as_ref().map(|t| t.join(", ")).unwrap_or_else(|| "-".to_string());
            let comment = archive.metadata.comment.as_deref().unwrap_or("-");
            let time_str = archive.metadata.time.format("%Y-%m-%d %H:%M:%S").to_string();

            println!(
                "{:<65} | {:<20} | {:<20} | {:<15} | {:<15} | {:<20}",
                archive.metadata.name,
                comment,
                tags,
                archive.metadata.hostname,
                archive.metadata.username,
                time_str
            );

            println!();
            println!("Files: {}", archive.stats.nfiles);
            println!("Original size: {}", super::format_size(archive.stats.original_size));
            println!("Deduplicated size: {}", super::format_size(archive.stats.deduplicated_size));
        }
        None => {
            let manifest = repo.load_manifest().await?;
            let descriptor = repo.descriptor();

            println!("Repository ID: {}", descriptor.id);
            println!("Location: {}", repo_path);
            println!("Encrypted: {}", descriptor.encrypted);
            println!("Archives: {}", manifest.archives.len());

            if !manifest.archives.is_empty() {
                let mut archives: Vec<_> = manifest.archives.iter().collect();
                // Sort by time ascending (oldest first)
                archives.sort_by(|a, b| a.time.cmp(&b.time));

                println!("    +");
                for archive in archives {
                    println!("    +--> {}", archive.name);
                }
            }
        }
    }

    Ok(())
}
