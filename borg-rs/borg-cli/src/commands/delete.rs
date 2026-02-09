//! Delete archives command

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, DeleteArgs};
use borg_core::error::BorgError;

pub async fn run(cli: &Cli, args: &DeleteArgs) -> Result<()> {
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

    let mut repo = open_repository(&repo_path).await?;

    for archive in &args.archives {
        if args.dry_run {
            println!("Would delete: {}", archive);
        } else {
            println!("Deleting: {}", archive);
            match repo.delete_archive(archive).await {
                Ok(_) => {},
                Err(BorgError::ArchiveNotFound { .. }) => eprintln!("Archive not found: {}", archive),
                Err(e) => anyhow::bail!("Failed to delete archive {}: {}", archive, e),
            }
        }
    }

    Ok(())
}
