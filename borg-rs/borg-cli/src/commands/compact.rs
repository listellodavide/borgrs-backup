//! Compact repository command

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, CompactArgs};

pub async fn run(cli: &Cli, args: &CompactArgs) -> Result<()> {
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

    let _repo = open_repository(&repo_path).await?;

    println!("Compacting repository: {}", repo_path);
    println!("Threshold: {}%", args.threshold);

    // Compact implementation

    Ok(())
}
