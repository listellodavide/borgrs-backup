//! Archive restoration command

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, RestoreArgs};

use borg_core::archive::ArchiveRestorer;

pub async fn run(cli: &Cli, args: &RestoreArgs) -> Result<()> {
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

    let restorer = ArchiveRestorer::new(&repo);

    // If paths are specified, use restore_paths, otherwise restore everything
    let stats = if args.paths.is_empty() {
        restorer.restore(&args.archive, &args.destination).await?
    } else {
        restorer.restore_paths(&args.archive, &args.destination, &args.paths).await?
    };

    println!("Restored archive: {}", args.archive);
    if !args.paths.is_empty() {
        println!("Paths requested: {}", args.paths.len());
    }
    println!("Files: {}", stats.files_restored);
    println!("Directories: {}", stats.dirs_restored);
    println!("Total bytes: {}", stats.bytes_restored);

    Ok(())
}
