//! Delete archives command

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, DeleteArgs};

pub async fn run(cli: &Cli, args: &DeleteArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;
    let mut repo = open_repository(&repo_path).await?;

    for archive in &args.archives {
        if args.dry_run {
            println!("Would delete: {}", archive);
        } else {
            println!("Deleting: {}", archive);
            // In the new architecture, deleting an archive means deleting its snapshot
            // We need to find the snapshot ID for the archive name first
            let manifest = repo.load_manifest().await?;
            if let Some(_archive_ref) = manifest.archives.iter().find(|a| a.name == *archive) {
                // We need to find the snapshot ID corresponding to this archive.
                // The manifest is constructed from snapshots, but ArchiveRef doesn't store the snapshot ID directly,
                // it stores the root_tree ID.
                // However, the Repository::load_manifest implementation iterates over snapshots.
                // We need a way to delete by name or expose delete_snapshot.

                // Since Repository doesn't expose delete_archive directly anymore, we need to implement it or use what's available.
                // The Repository struct has an `engine` field which is private.
                // We should add a `delete_archive` method to Repository in borg-core.

                // For now, let's assume we can add it to Repository.
                // But since I can't modify borg-core in this step easily without context switching,
                // I will check if I can implement it here or if I missed it.

                // Wait, I can modify borg-core. I should add delete_archive to Repository.
                // But first let me comment this out or mark as todo if I can't fix it immediately,
                // OR better, I will add the method to Repository in the next step.

                // Let's assume the method exists for now and I will add it to borg-core/src/repository.rs
                repo.delete_archive(archive).await
                    .map_err(|e| anyhow::anyhow!("Failed to delete archive {}: {}", archive, e))?;
            } else {
                eprintln!("Archive not found: {}", archive);
            }
        }
    }

    Ok(())
}
