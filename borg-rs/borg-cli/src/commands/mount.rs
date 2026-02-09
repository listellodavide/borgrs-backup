//! Mount archive as FUSE filesystem

use anyhow::Result;
use super::get_repo_path;
use crate::{Cli, MountArgs};

pub async fn run(cli: &Cli, args: &MountArgs) -> Result<()> {
    let repo_path_raw = get_repo_path(cli)?;

    // Normalize WebDAV URL if credentials are provided (either from CLI or if it's a remote repo)
    let _repo_path = if args.webdav_user.is_some() || args.webdav_pass.is_some() {
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

    println!("Mounting {} at {:?}", 
        args.archive.as_deref().unwrap_or("repository"),
        args.mountpoint
    );

    // FUSE mount implementation would go here
    // This requires the fuser crate

    Ok(())
}
