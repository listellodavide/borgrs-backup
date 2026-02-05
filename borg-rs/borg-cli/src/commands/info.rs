//! Repository/archive info command

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, InfoArgs};

use borg_core::archive::ArchiveExtractor;

pub async fn run(cli: &Cli, args: &InfoArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;
    let repo = open_repository(&repo_path).await?;

    match &args.archive {
        Some(archive_name) => {
            let extractor = ArchiveExtractor::new(&repo);
            let archive = extractor.load_archive(archive_name).await?;
            
            println!("Archive: {}", archive.metadata.name);
            println!("Time: {}", archive.metadata.time);
            println!("Hostname: {}", archive.metadata.hostname);
            println!("Username: {}", archive.metadata.username);
            println!("Comment: {}", archive.metadata.comment.as_deref().unwrap_or("-"));
            println!();
            println!("Files: {}", archive.stats.nfiles);
            println!("Original size: {}", super::format_size(archive.stats.original_size));
            println!("Deduplicated size: {}", super::format_size(archive.stats.deduplicated_size));
        }
        None => {
            let manifest = repo.load_manifest().await?;
            println!("Repository ID: {}", manifest.repository_id);
            println!("Location: {}", repo_path);
            println!("Encrypted: {}", repo.config().encrypted);
            println!("Archives: {}", manifest.archives.len());
        }
    }

    Ok(())
}
