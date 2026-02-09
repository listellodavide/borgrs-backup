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

    // Check for the special '::original' path marker
    let use_original_paths = args.paths.iter().any(|p| p.to_string_lossy() == "::original");

    let stats = if use_original_paths {
        // Load archive to get original paths
        let archive = restorer.load_archive(&args.archive).await?;
        if let Some(original_paths) = &archive.metadata.original_paths {
            println!("Found original paths in metadata: {:?}", original_paths);

            if original_paths.is_empty() {
                 anyhow::bail!("Archive metadata contains empty original paths list");
            }

            if original_paths.len() > 1 {
                // If multiple roots, we can't easily map items back to their absolute paths
                // because we only stored relative paths in the archive items.
                // We fallback to restoring everything to the specified destination.
                println!("Warning: Archive has multiple source paths. Restoring all files to '{}'.", args.destination.display());
                restorer.restore(&args.archive, &args.destination).await?
            } else {
                // Single root: we can restore everything to that root.
                let original_root = &original_paths[0];
                println!("Restoring to original location: {}", original_root.display());
                restorer.restore(&args.archive, original_root).await?
            }
        } else {
            anyhow::bail!("Archive does not contain original path information");
        }
    } else if args.paths.is_empty() {
        restorer.restore(&args.archive, &args.destination).await?
    } else {
        restorer.restore_paths(&args.archive, &args.destination, &args.paths).await?
    };

    println!("Restored archive: {}", args.archive);
    if !args.paths.is_empty() && !use_original_paths {
        println!("Paths requested: {}", args.paths.len());
    }
    println!("Files: {}", stats.files_restored);
    println!("Directories: {}", stats.dirs_restored);
    println!("Total bytes: {}", stats.bytes_restored);

    Ok(())
}
